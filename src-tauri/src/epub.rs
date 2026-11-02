// Fixed-layout comic EPUB in the shape KindleGen expects (same metadata KCC writes).

use crate::imaging::{Page, DH, DW};
use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::Path;
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipWriter};

pub fn build(path: &Path, title: &str, uuid: &str, pages: &[Page], rtl: bool) -> Result<(), String> {
    let file = File::create(path).map_err(|e| e.to_string())?;
    let mut z = ZipWriter::new(BufWriter::new(file));
    let opt = SimpleFileOptions::default()
        .compression_method(CompressionMethod::Stored)
        .large_file(false);
    let put = |z: &mut ZipWriter<BufWriter<File>>, name: &str, data: &[u8]| -> Result<(), String> {
        z.start_file(name, opt).map_err(|e| e.to_string())?;
        z.write_all(data).map_err(|e| e.to_string())
    };

    put(&mut z, "mimetype", b"application/epub+zip")?;
    put(&mut z, "META-INF/container.xml", CONTAINER.as_bytes())?;
    put(&mut z, "OEBPS/Text/style.css", CSS.as_bytes())?;

    let title = esc(title);
    for (i, p) in pages.iter().enumerate() {
        let n = i + 1;
        let html = format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE html>
<html xmlns="http://www.w3.org/1999/xhtml" xmlns:epub="http://www.idpf.org/2007/ops">
<head>
<title>{n}</title>
<link href="style.css" type="text/css" rel="stylesheet"/>
<meta name="viewport" content="width={w}, height={h}"/>
</head>
<body>
<div style="text-align:center;top:0.0%;">
<img width="{w}" height="{h}" src="../Images/p{n:05}.jpg"/>
</div>
</body>
</html>
"#,
            w = p.w,
            h = p.h
        );
        put(&mut z, &format!("OEBPS/Text/p{n:05}.xhtml"), html.as_bytes())?;
        let img = std::fs::read(&p.path).map_err(|e| e.to_string())?;
        put(&mut z, &format!("OEBPS/Images/p{n:05}.jpg"), &img)?;
        if i == 0 {
            put(&mut z, "OEBPS/Images/cover.jpg", &img)?;
        }
    }

    let mut manifest = String::new();
    let mut spine = String::new();
    for i in 1..=pages.len() {
        manifest.push_str(&format!(
            "<item id=\"page_{i}\" href=\"Text/p{i:05}.xhtml\" media-type=\"application/xhtml+xml\"/>\n\
             <item id=\"img_{i}\" href=\"Images/p{i:05}.jpg\" media-type=\"image/jpeg\"/>\n"
        ));
        spine.push_str(&format!("<itemref idref=\"page_{i}\" linear=\"yes\"/>\n"));
    }
    let (mode, dir) = if rtl { ("horizontal-rl", "rtl") } else { ("horizontal-lr", "ltr") };
    let opf = format!(
        r##"<?xml version="1.0" encoding="UTF-8"?>
<package version="3.0" unique-identifier="BookID" xmlns="http://www.idpf.org/2007/opf">
<metadata xmlns:opf="http://www.idpf.org/2007/opf" xmlns:dc="http://purl.org/dc/elements/1.1/">
<dc:title>{title}</dc:title>
<dc:language>en-US</dc:language>
<dc:identifier id="BookID">urn:uuid:{uuid}</dc:identifier>
<dc:contributor id="contributor">Kindle Manga Converter</dc:contributor>
<dc:creator>Unknown</dc:creator>
<meta property="dcterms:modified">{now}</meta>
<meta name="cover" content="cover"/>
<meta name="fixed-layout" content="true"/>
<meta name="original-resolution" content="{DW}x{DH}"/>
<meta name="book-type" content="comic"/>
<meta name="primary-writing-mode" content="{mode}"/>
<meta name="zero-gutter" content="true"/>
<meta name="zero-margin" content="true"/>
<meta name="ke-border-color" content="#FFFFFF"/>
<meta name="ke-border-width" content="0"/>
<meta name="orientation-lock" content="none"/>
<meta name="region-mag" content="true"/>
<meta property="rendition:spread">none</meta>
<meta property="rendition:layout">pre-paginated</meta>
</metadata>
<manifest>
<item id="ncx" href="toc.ncx" media-type="application/x-dtbncx+xml"/>
<item id="nav" href="Text/nav.xhtml" media-type="application/xhtml+xml" properties="nav"/>
<item id="css" href="Text/style.css" media-type="text/css"/>
<item id="cover" href="Images/cover.jpg" media-type="image/jpeg" properties="cover-image"/>
{manifest}</manifest>
<spine page-progression-direction="{dir}" toc="ncx">
{spine}</spine>
</package>
"##,
        now = now_utc()
    );
    put(&mut z, "OEBPS/content.opf", opf.as_bytes())?;

    let ncx = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<ncx version="2005-1" xml:lang="en-US" xmlns="http://www.daisy.org/z3986/2005/ncx/">
<head>
<meta name="dtb:uid" content="urn:uuid:{uuid}"/>
<meta name="dtb:depth" content="1"/>
<meta name="dtb:totalPageCount" content="0"/>
<meta name="dtb:maxPageNumber" content="0"/>
<meta name="generated" content="true"/>
</head>
<docTitle><text>{title}</text></docTitle>
<navMap>
<navPoint id="Text" playOrder="1"><navLabel><text>{title}</text></navLabel><content src="Text/p00001.xhtml"/></navPoint>
</navMap>
</ncx>
"#
    );
    put(&mut z, "OEBPS/toc.ncx", ncx.as_bytes())?;

    let nav = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE html>
<html xmlns="http://www.w3.org/1999/xhtml" xmlns:epub="http://www.idpf.org/2007/ops">
<head>
<title>{title}</title>
<meta charset="utf-8"/>
</head>
<body>
<nav xmlns:epub="http://www.idpf.org/2007/ops" epub:type="toc" id="toc">
<ol>
<li><a href="p00001.xhtml">{title}</a></li>
</ol>
</nav>
</body>
</html>
"#
    );
    put(&mut z, "OEBPS/Text/nav.xhtml", nav.as_bytes())?;

    z.finish().map_err(|e| e.to_string())?.flush().map_err(|e| e.to_string())
}

const CONTAINER: &str = r#"<?xml version="1.0"?>
<container version="1.0" xmlns="urn:oasis:names:tc:opendocument:xmlns:container">
<rootfiles>
<rootfile full-path="OEBPS/content.opf" media-type="application/oebps-package+xml"/>
</rootfiles>
</container>
"#;

const CSS: &str = "@page {\nmargin: 0;\n}\nbody {\ndisplay: block;\nmargin: 0;\npadding: 0;\n}\n";

fn esc(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn now_utc() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let (days, rem) = (secs.div_euclid(86400), secs.rem_euclid(86400));
    // civil-from-days (Howard Hinnant)
    let z = days + 719468;
    let era = z.div_euclid(146097);
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + if m <= 2 { 1 } else { 0 };
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z",
        rem / 3600,
        rem % 3600 / 60,
        rem % 60
    )
}
