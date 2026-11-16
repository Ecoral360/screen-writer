//! Small integer/float geometry helpers.

#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct Pt {
    pub x: f32,
    pub y: f32,
}

impl Pt {
    pub const fn new(x: f32, y: f32) -> Self {
        Pt { x, y }
    }
    pub fn dist(self, o: Pt) -> f32 {
        ((self.x - o.x).powi(2) + (self.y - o.y).powi(2)).sqrt()
    }
}

/// Distance from point `p` to segment `a`-`b`.
pub fn seg_dist(p: Pt, a: Pt, b: Pt) -> f32 {
    let (dx, dy) = (b.x - a.x, b.y - a.y);
    let len2 = dx * dx + dy * dy;
    if len2 < 1e-6 {
        return p.dist(a);
    }
    let t = (((p.x - a.x) * dx + (p.y - a.y) * dy) / len2).clamp(0.0, 1.0);
    p.dist(Pt::new(a.x + t * dx, a.y + t * dy))
}

/// Half-open integer rectangle `[x0, x1) x [y0, y1)`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct IRect {
    pub x0: i32,
    pub y0: i32,
    pub x1: i32,
    pub y1: i32,
}

impl IRect {
    pub const EMPTY: IRect = IRect { x0: 0, y0: 0, x1: 0, y1: 0 };

    pub const fn new(x0: i32, y0: i32, x1: i32, y1: i32) -> Self {
        IRect { x0, y0, x1, y1 }
    }
    pub fn xywh(x: i32, y: i32, w: i32, h: i32) -> Self {
        IRect::new(x, y, x + w, y + h)
    }
    /// Bounding box of float coordinates, expanded by `pad` pixels.
    pub fn from_f(x0: f32, y0: f32, x1: f32, y1: f32, pad: f32) -> Self {
        IRect::new(
            (x0.min(x1) - pad).floor() as i32,
            (y0.min(y1) - pad).floor() as i32,
            (x0.max(x1) + pad).ceil() as i32 + 1,
            (y0.max(y1) + pad).ceil() as i32 + 1,
        )
    }
    pub fn around(p: Pt, r: f32) -> Self {
        IRect::from_f(p.x, p.y, p.x, p.y, r)
    }
    pub fn from_points(pts: &[Pt], pad: f32) -> Self {
        let mut it = pts.iter();
        let Some(f) = it.next() else { return IRect::EMPTY };
        let (mut x0, mut y0, mut x1, mut y1) = (f.x, f.y, f.x, f.y);
        for p in it {
            x0 = x0.min(p.x);
            y0 = y0.min(p.y);
            x1 = x1.max(p.x);
            y1 = y1.max(p.y);
        }
        IRect::from_f(x0, y0, x1, y1, pad)
    }
    pub fn w(&self) -> i32 {
        self.x1 - self.x0
    }
    pub fn h(&self) -> i32 {
        self.y1 - self.y0
    }
    pub fn is_empty(&self) -> bool {
        self.x1 <= self.x0 || self.y1 <= self.y0
    }
    pub fn area(&self) -> i64 {
        if self.is_empty() { 0 } else { self.w() as i64 * self.h() as i64 }
    }
    pub fn union(&self, o: &IRect) -> IRect {
        if self.is_empty() {
            return *o;
        }
        if o.is_empty() {
            return *self;
        }
        IRect::new(self.x0.min(o.x0), self.y0.min(o.y0), self.x1.max(o.x1), self.y1.max(o.y1))
    }
    pub fn intersect(&self, o: &IRect) -> IRect {
        let r = IRect::new(self.x0.max(o.x0), self.y0.max(o.y0), self.x1.min(o.x1), self.y1.min(o.y1));
        if r.is_empty() { IRect::EMPTY } else { r }
    }
    pub fn touches(&self, o: &IRect, slack: i32) -> bool {
        self.x0 - slack < o.x1 && o.x0 - slack < self.x1 && self.y0 - slack < o.y1 && o.y0 - slack < self.y1
    }
    pub fn contains(&self, x: f32, y: f32) -> bool {
        x >= self.x0 as f32 && x < self.x1 as f32 && y >= self.y0 as f32 && y < self.y1 as f32
    }
    pub fn translate(&self, dx: i32, dy: i32) -> IRect {
        IRect::new(self.x0 + dx, self.y0 + dy, self.x1 + dx, self.y1 + dy)
    }
}

/// Accumulates damaged regions, merging overlapping rectangles.
#[derive(Default)]
pub struct Damage {
    rects: Vec<IRect>,
}

impl Damage {
    pub fn add(&mut self, r: IRect) {
        if !r.is_empty() {
            self.rects.push(r);
        }
    }

    /// Returns a merged, clipped list of rectangles and resets the accumulator.
    pub fn take(&mut self, screen: IRect) -> Vec<IRect> {
        let mut rects: Vec<IRect> =
            self.rects.drain(..).map(|r| r.intersect(&screen)).filter(|r| !r.is_empty()).collect();
        // Merge rectangles that touch, until stable.
        let mut changed = true;
        while changed {
            changed = false;
            let mut i = 0;
            while i < rects.len() {
                let mut j = i + 1;
                while j < rects.len() {
                    let (a, b) = (rects[i], rects[j]);
                    let u = a.union(&b);
                    // Merge when overlapping, or when the union wastes little area.
                    if a.touches(&b, 8) || u.area() <= (a.area() + b.area()) * 5 / 4 {
                        rects[i] = u;
                        rects.swap_remove(j);
                        changed = true;
                    } else {
                        j += 1;
                    }
                }
                i += 1;
            }
        }
        let total: i64 = rects.iter().map(|r| r.area()).sum();
        if total > screen.area() * 6 / 10 {
            return vec![screen];
        }
        rects
    }

    pub fn is_empty(&self) -> bool {
        self.rects.is_empty()
    }
}
