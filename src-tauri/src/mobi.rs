// Post-processing of KindleGen's dual MOBI (MOBI7 + KF8 in one file):
// - mark both headers as personal document (EXTH 501 PDOC) with an ASIN (EXTH 113), as KCC does
// - for AZW3, cut out the KF8 part into a standalone file (port of KindleUnpack's mobi_split)

const UNIQUE_ID_SEED: usize = 68;
const NUM_RECORDS: usize = 76;
const FIRST_RECORD: usize = 78;

const MOBI_HEADER_LENGTH: usize = 20;
const MOBI_VERSION: usize = 36;
const TITLE_OFFSET: usize = 84;
const FIRST_RESC_RECORD: usize = 108;
const LAST_CONTENT_INDEX: usize = 194;
const KF8_FDST_INDEX: usize = 192;
const FCIS_INDEX: usize = 200;
const FLIS_INDEX: usize = 208;
const DATP_INDEX: usize = 256;
const HUFF_TBL_OFF: usize = 120;
const FLAGS: usize = 0x80;

struct Pdb {
    header: Vec<u8>,
    recs: Vec<Vec<u8>>,
}

fn be16(d: &[u8], o: usize) -> Result<usize, String> {
    d.get(o..o + 2)
        .map(|b| u16::from_be_bytes([b[0], b[1]]) as usize)
        .ok_or_else(|| "MOBI file is too short".to_string())
}

fn be32(d: &[u8], o: usize) -> Result<u32, String> {
    d.get(o..o + 4)
        .map(|b| u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
        .ok_or_else(|| "MOBI file is too short".to_string())
}

fn put32(d: &mut [u8], o: usize, v: u32) {
    d[o..o + 4].copy_from_slice(&v.to_be_bytes());
}

impl Pdb {
    fn parse(d: &[u8]) -> Result<Pdb, String> {
        let n = be16(d, NUM_RECORDS)?;
        let mut offs = Vec::with_capacity(n + 1);
        for i in 0..n {
            offs.push(be32(d, FIRST_RECORD + i * 8)? as usize);
        }
        offs.push(d.len());
        let mut recs = Vec::with_capacity(n);
        for i in 0..n {
            let (a, b) = (offs[i], offs[i + 1]);
            if a > b || b > d.len() {
                return Err("MOBI record table is broken".into());
            }
            recs.push(d[a..b].to_vec());
        }
        Ok(Pdb { header: d[..FIRST_RECORD].to_vec(), recs })
    }

    fn build(&self) -> Vec<u8> {
        let n = self.recs.len();
        let mut out = self.header.clone();
        put32(&mut out, UNIQUE_ID_SEED, (2 * n + 1) as u32);
        out[NUM_RECORDS..NUM_RECORDS + 2].copy_from_slice(&(n as u16).to_be_bytes());
        let mut ofs = FIRST_RECORD + 8 * n + 2;
        for (i, r) in self.recs.iter().enumerate() {
            out.extend_from_slice(&(ofs as u32).to_be_bytes());
            out.extend_from_slice(&((2 * i) as u32).to_be_bytes());
            ofs += r.len();
        }
        out.extend_from_slice(&[0, 0]);
        for r in &self.recs {
            out.extend_from_slice(r);
        }
        out
    }
}

// ---------- EXTH ----------

fn exth_params(r: &[u8]) -> Result<(usize, usize, usize), String> {
    let base = 16 + be32(r, MOBI_HEADER_LENGTH)? as usize;
    if r.get(base..base + 4) != Some(b"EXTH") {
        return Err("EXTH block not found in MOBI header".into());
    }
    Ok((base, be32(r, base + 4)? as usize, be32(r, base + 8)? as usize))
}

fn read_exth(r: &[u8], id: u32) -> Result<Vec<Vec<u8>>, String> {
    let (base, _, count) = exth_params(r)?;
    let mut p = base + 12;
    let mut vals = Vec::new();
    for _ in 0..count {
        let eid = be32(r, p)?;
        let size = be32(r, p + 4)? as usize;
        if size < 8 || p + size > r.len() {
            return Err("EXTH entry is broken".into());
        }
        if eid == id {
            vals.push(r[p + 8..p + size].to_vec());
        }
        p += size;
    }
    Ok(vals)
}

fn set_exth_counts(r: &mut Vec<u8>, base: usize, len: usize, count: usize, title_delta: isize) -> Result<(), String> {
    put32(r, base + 4, len as u32);
    put32(r, base + 8, count as u32);
    let t = be32(r, TITLE_OFFSET)? as isize + title_delta;
    put32(r, TITLE_OFFSET, t as u32);
    Ok(())
}

/// Remove the first entry with this id.
fn del_exth(r: &[u8], id: u32) -> Result<Vec<u8>, String> {
    let (base, len, count) = exth_params(r)?;
    let mut p = base + 12;
    for _ in 0..count {
        let eid = be32(r, p)?;
        let size = be32(r, p + 4)? as usize;
        if size < 8 || p + size > r.len() {
            return Err("EXTH entry is broken".into());
        }
        if eid == id {
            let mut out = Vec::with_capacity(r.len());
            out.extend_from_slice(&r[..p]);
            out.extend_from_slice(&r[p + size..]);
            set_exth_counts(&mut out, base, len - size, count - 1, -(size as isize))?;
            return Ok(out);
        }
        p += size;
    }
    Ok(r.to_vec())
}

fn add_exth(r: &[u8], id: u32, data: &[u8]) -> Result<Vec<u8>, String> {
    let (base, len, count) = exth_params(r)?;
    let size = 8 + data.len();
    let mut out = Vec::with_capacity(r.len() + size);
    out.extend_from_slice(&r[..base + 12]);
    out.extend_from_slice(&id.to_be_bytes());
    out.extend_from_slice(&(size as u32).to_be_bytes());
    out.extend_from_slice(data);
    out.extend_from_slice(&r[base + 12..]);
    set_exth_counts(&mut out, base, len + size, count + 1, size as isize)?;
    Ok(out)
}

/// Replace the value of the first entry with this id (no-op when missing).
fn write_exth(r: &[u8], id: u32, data: &[u8]) -> Result<Vec<u8>, String> {
    if read_exth(r, id)?.is_empty() {
        return Ok(r.to_vec());
    }
    // the entry moves to the front of the list; order of EXTH entries does not matter
    add_exth(&del_exth(r, id)?, id, data)
}

fn fix_meta(r: &[u8], asin: &str) -> Result<Vec<u8>, String> {
    let r = del_exth(r, 501)?;
    let r = del_exth(&r, 113)?;
    let r = add_exth(&r, 501, b"PDOC")?;
    add_exth(&r, 113, asin.as_bytes())
}

// ---------- public ----------

pub fn finish(data: &[u8], asin: &str, azw3: bool) -> Result<Vec<u8>, String> {
    let mut pdb = Pdb::parse(data)?;
    if pdb.recs.is_empty() {
        return Err("MOBI file has no records".into());
    }
    pdb.recs[0] = fix_meta(&pdb.recs[0], asin)?;
    let ver = be32(&pdb.recs[0], MOBI_VERSION)?;
    let mut kf8 = None;
    if ver != 8 {
        if let Some(v) = read_exth(&pdb.recs[0], 121)?.first() {
            let k = be32(v, 0)?;
            if k != 0xffff_ffff && (k as usize) < pdb.recs.len() {
                kf8 = Some(k as usize);
            }
        }
    }
    if let Some(k) = kf8 {
        pdb.recs[k] = fix_meta(&pdb.recs[k], asin)?;
    }
    if !azw3 {
        return Ok(pdb.build());
    }
    let Some(k) = kf8 else {
        if ver == 8 {
            return Ok(pdb.build()); // already a pure KF8 book
        }
        return Err("KindleGen made an old-style MOBI without a KF8 part, so no AZW3 can be made from it.".into());
    };

    // standalone KF8: records from the KF8 header to the end, with the shared images put back in
    let rec0 = &pdb.recs[0];
    let first = be32(rec0, FIRST_RESC_RECORD)? as usize;
    let mut last = be16(rec0, LAST_CONTENT_INDEX)?;
    if last == 0xffff {
        for ofs in [FCIS_INDEX, FLIS_INDEX, DATP_INDEX, HUFF_TBL_OFF] {
            let n = be32(rec0, ofs)? as usize;
            if n > 0 && n < last {
                last = n - 1;
            }
        }
    }
    if first > last || last >= k {
        return Err("Could not find the pictures inside the MOBI file.".into());
    }
    let count = last - first + 1;
    let mut recs: Vec<Vec<u8>> = pdb.recs[k..].to_vec();
    let target = be32(&recs[0], FIRST_RESC_RECORD)? as usize;
    if target > recs.len() {
        return Err("KF8 header points outside the file.".into());
    }
    recs.splice(target..target, pdb.recs[first..=last].iter().cloned());

    let mut r0 = recs[0].clone();
    while read_exth(&r0, 116)?.len() > 1 {
        r0 = del_exth(&r0, 116)?;
    }
    r0 = write_exth(&r0, 125, &(count as u32).to_be_bytes())?;
    let flags = (be32(&r0, FLAGS)? & 0x1FFF) | 0x0800;
    put32(&mut r0, FLAGS, flags);
    for ofs in [KF8_FDST_INDEX, FCIS_INDEX, FLIS_INDEX, DATP_INDEX, HUFF_TBL_OFF] {
        let n = be32(&r0, ofs)?;
        if n != 0xffff_ffff {
            put32(&mut r0, ofs, n + count as u32);
        }
    }
    recs[0] = r0;
    Ok(Pdb { header: pdb.header, recs }.build())
}
