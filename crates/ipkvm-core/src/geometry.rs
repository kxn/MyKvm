#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Point {
    pub x: i32,
    pub y: i32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ViewRect {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Ch9329Point {
    pub x: u16,
    pub y: u16,
}

pub fn map_pointer_to_ch9329(point: Point, rect: ViewRect) -> Ch9329Point {
    Ch9329Point {
        x: map_axis(point.x, rect.x, rect.width),
        y: map_axis(point.y, rect.y, rect.height),
    }
}

fn map_axis(value: i32, origin: i32, size: u32) -> u16 {
    if size == 0 {
        return 0;
    }

    let relative = i64::from(value - origin).clamp(0, i64::from(size));
    ((relative * 4095) / i64::from(size)) as u16
}
