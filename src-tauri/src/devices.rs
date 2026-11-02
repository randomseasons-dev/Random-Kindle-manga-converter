// Kindle profiles, same list and screen sizes as KCC.

use serde::Serialize;

#[derive(Serialize)]
pub struct Device {
    pub id: &'static str,
    pub name: &'static str,
    pub group: &'static str,
    pub w: u32,
    pub h: u32,
    /// colour screen: colour pages stay in colour
    pub color: bool,
    /// old firmware without KF8: MOBI only, no AZW3
    pub kf7: bool,
}

const NEW: &str = "Kindles";
const OLD: &str = "Older Kindles";
const SIZE: &str = "Other Kindles by screen size";

const fn d(id: &'static str, name: &'static str, group: &'static str, w: u32, h: u32, color: bool, kf7: bool) -> Device {
    Device { id, name, group, w, h, color, kf7 }
}

pub const DEVICES: &[Device] = &[
    d("K11", "Kindle 11/2026", NEW, 1072, 1448, false, false),
    d("KPW5", "Kindle Paperwhite 5/Signature Edition", NEW, 1236, 1648, false, false),
    d("KPW6", "Kindle Paperwhite 6/2026", NEW, 1272, 1696, false, false),
    d("KCS", "Kindle Colorsoft", NEW, 1272, 1696, true, false),
    d("KO", "Kindle Oasis 2/3", NEW, 1264, 1680, false, false),
    d("KS", "Kindle Scribe 1/2", NEW, 1860, 2480, false, false),
    d("KS3", "Kindle Scribe 3", NEW, 1986, 2648, false, false),
    d("KSCS", "Kindle Scribe Colorsoft", NEW, 1986, 2648, true, false),
    d("KPW34", "Kindle Paperwhite 3/4/Oasis", OLD, 1072, 1448, false, false),
    d("KV", "Kindle Voyage", OLD, 1072, 1448, false, false),
    d("K810", "Kindle 8/10", OLD, 600, 800, false, false),
    d("KPW", "Kindle Paperwhite 1/2", OLD, 758, 1024, false, false),
    d("K57", "Kindle 5/7", OLD, 600, 800, false, false),
    d("K34", "Kindle Keyboard/Touch", OLD, 600, 800, false, true),
    d("KDX", "Kindle DX/DXG", OLD, 824, 1000, false, true),
    d("K2", "Kindle 2", OLD, 600, 670, false, true),
    d("K1", "Kindle 1", OLD, 600, 670, false, true),
    d("K1920", "Kindle 1440x1920", SIZE, 1440, 1920, false, false),
    d("KS1240", "Kindle 1240", SIZE, 1240, 1860, false, false),
    d("KS1324", "Kindle 1324", SIZE, 1324, 1986, false, false),
    d("KS1860", "Kindle 1860", SIZE, 1860, 1920, false, false),
    d("KS1920", "Kindle 1920", SIZE, 1920, 1920, false, false),
];

impl Device {
    /// Scribe-class profiles: KindleGen only takes pictures up to 1920 px,
    /// so pages are at most 1920 wide and taller pages are stored as two stacked pictures.
    pub fn scribe(&self) -> bool {
        self.id.starts_with("KS")
    }

    /// Size the pages are fitted to.
    pub fn size(&self) -> (u32, u32) {
        if self.scribe() {
            (self.w.min(1920), self.h)
        } else {
            (self.w, self.h)
        }
    }

    pub fn jpeg_quality(&self) -> u8 {
        if self.scribe() || self.id == "KCS" {
            90
        } else {
            85
        }
    }
}

pub fn get(id: &str) -> &'static Device {
    DEVICES.iter().find(|d| d.id == id).unwrap_or(&DEVICES[0])
}
