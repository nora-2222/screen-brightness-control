#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ControlMethod {
    /// Hardware brightness via DDC/CI (saves power).
    DdcCi,
    /// Software dimming overlay (reduces glare only).
    Overlay,
}

impl ControlMethod {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::DdcCi => "ddc/ci",
            Self::Overlay => "overlay",
        }
    }
}

#[derive(Debug, Clone)]
pub struct MonitorInfo {
    pub id: String,
    pub name: String,
    pub bounds: Rect,
    pub method: ControlMethod,
    pub min_brightness: u8,
    pub max_brightness: u8,
}
