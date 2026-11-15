//! Binary space partitioning tree for the Hyprland-style `dwindle` layout.
//!
//! Each leaf is a window. A new window splits an existing leaf in two (the one
//! under the cursor, or the last focused one); closing a window gives its
//! space back to its sibling. Whether a split is side by side or on top of
//! each other is decided from the shape of the space when laying out, like
//! Hyprland: wide spaces split left/right, tall ones top/bottom.

pub type Win = u64;
pub type Rect = (i32, i32, i32, i32);

#[derive(Debug, Clone, PartialEq)]
pub enum Node {
    Leaf(Win),
    Split { ratio: f32, a: Box<Node>, b: Box<Node> },
}

impl Node {
    pub fn leaves(&self, out: &mut Vec<Win>) {
        match self {
            Node::Leaf(w) => out.push(*w),
            Node::Split { a, b, .. } => {
                a.leaves(out);
                b.leaves(out);
            }
        }
    }

    pub fn contains(&self, w: Win) -> bool {
        match self {
            Node::Leaf(x) => *x == w,
            Node::Split { a, b, .. } => a.contains(w) || b.contains(w),
        }
    }

    /// Split the leaf `target` in two; `new` goes first (left/top) if `first`.
    pub fn insert(&mut self, target: Win, new: Win, first: bool) -> bool {
        match self {
            Node::Leaf(w) if *w == target => {
                let (a, b) = if first { (new, target) } else { (target, new) };
                *self = Node::Split { ratio: 0.5, a: Box::new(Node::Leaf(a)), b: Box::new(Node::Leaf(b)) };
                true
            }
            Node::Leaf(_) => false,
            Node::Split { a, b, .. } => a.insert(target, new, first) || b.insert(target, new, first),
        }
    }

    /// Remove a window; its sibling takes over the parent's space.
    /// Returns None when the tree becomes empty.
    pub fn remove(self, w: Win) -> Option<Node> {
        match self {
            Node::Leaf(x) if x == w => None,
            leaf @ Node::Leaf(_) => Some(leaf),
            Node::Split { ratio, a, b } => match (a.remove(w), b.remove(w)) {
                (Some(a), Some(b)) => Some(Node::Split { ratio, a: Box::new(a), b: Box::new(b) }),
                (Some(only), None) | (None, Some(only)) => Some(only),
                (None, None) => None,
            },
        }
    }

    /// Swap two windows' places.
    pub fn swap(&mut self, x: Win, y: Win) {
        match self {
            Node::Leaf(w) => {
                if *w == x {
                    *w = y;
                } else if *w == y {
                    *w = x;
                }
            }
            Node::Split { a, b, .. } => {
                a.swap(x, y);
                b.swap(x, y);
            }
        }
    }

    /// Grow (`delta` > 0) or shrink the window `w` by moving the split it
    /// belongs to. With `absolute`, set that split's ratio instead.
    pub fn resize(&mut self, w: Win, delta: f32, absolute: bool) -> bool {
        let Node::Split { ratio, a, b } = self else { return false };
        // the deepest split wins: try the children first
        if a.resize(w, delta, absolute) || b.resize(w, delta, absolute) {
            return true;
        }
        let in_a = matches!(**a, Node::Leaf(x) if x == w);
        let in_b = matches!(**b, Node::Leaf(x) if x == w);
        if !in_a && !in_b {
            return false;
        }
        let new = if absolute {
            if in_a { delta } else { 1.0 - delta }
        } else if in_a {
            *ratio + delta
        } else {
            *ratio - delta
        };
        *ratio = new.clamp(0.1, 0.9);
        true
    }

    /// Lay the tree out in `r`, with `ih`/`iv` px between windows stacked
    /// vertically/side by side.
    pub fn layout(&self, r: Rect, ih: i32, iv: i32, out: &mut Vec<(Win, Rect)>) {
        match self {
            Node::Leaf(w) => out.push((*w, r)),
            Node::Split { ratio, a, b } => {
                let (x, y, w, h) = r;
                if w >= h {
                    let first = (((w - iv) as f32) * ratio) as i32;
                    a.layout((x, y, first.max(1), h), ih, iv, out);
                    b.layout((x + first + iv, y, (w - first - iv).max(1), h), ih, iv, out);
                } else {
                    let first = (((h - ih) as f32) * ratio) as i32;
                    a.layout((x, y, w, first.max(1)), ih, iv, out);
                    b.layout((x, y + first + ih, w, (h - first - ih).max(1)), ih, iv, out);
                }
            }
        }
    }
}

/// Which side of `rect` the point is on, for a split of that rect:
/// true = the first half (left or top).
pub fn first_half(rect: Rect, px: i32, py: i32) -> bool {
    let (x, y, w, h) = rect;
    if w >= h { px < x + w / 2 } else { py < y + h / 2 }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rects(n: &Node) -> Vec<(Win, Rect)> {
        let mut out = vec![];
        n.layout((0, 0, 1000, 600), 0, 0, &mut out);
        out
    }

    #[test]
    fn new_window_splits_the_chosen_one() {
        let mut t = Node::Leaf(1);
        t.insert(1, 2, false); // 1 | 2
        t.insert(1, 3, false); // window 1 splits into 1 / 3, 2 keeps its half
        let r = rects(&t);
        assert_eq!(r, vec![(1, (0, 0, 500, 300)), (3, (0, 300, 500, 300)), (2, (500, 0, 500, 600))]);
    }

    #[test]
    fn cursor_side_decides_order() {
        let mut t = Node::Leaf(1);
        t.insert(1, 2, true); // cursor on the left half: new window goes left
        assert_eq!(rects(&t)[0], (2, (0, 0, 500, 600)));
        assert!(first_half((0, 0, 1000, 600), 100, 300));
        assert!(!first_half((0, 0, 1000, 600), 900, 300));
        assert!(!first_half((0, 0, 400, 600), 100, 500)); // tall: bottom half
    }

    #[test]
    fn closing_gives_space_back() {
        let mut t = Node::Leaf(1);
        t.insert(1, 2, false);
        t.insert(1, 3, false);
        let t = t.remove(3).unwrap();
        assert_eq!(rects(&t), vec![(1, (0, 0, 500, 600)), (2, (500, 0, 500, 600))]);
        let t = t.remove(1).unwrap();
        assert_eq!(rects(&t), vec![(2, (0, 0, 1000, 600))]);
        assert_eq!(t.remove(2), None);
    }

    #[test]
    fn swap_and_resize() {
        let mut t = Node::Leaf(1);
        t.insert(1, 2, false);
        t.swap(1, 2);
        assert_eq!(rects(&t)[0].0, 2);
        assert!(t.resize(2, 0.1, false)); // 2 is now first: grows
        assert_eq!(rects(&t)[0], (2, (0, 0, 600, 600)));
        assert!(t.resize(1, 0.1, false)); // 1 is second: growing it shrinks the ratio
        assert_eq!(rects(&t)[0], (2, (0, 0, 500, 600)));
        assert!(!t.resize(99, 0.1, false));
    }

    #[test]
    fn gaps_and_no_overlap() {
        let mut t = Node::Leaf(1);
        for (i, new) in (2..=7).enumerate() {
            t.insert(1 + i as u64, new, i % 2 == 0);
        }
        let mut out = vec![];
        t.layout((10, 10, 1900, 1000), 10, 10, &mut out);
        assert_eq!(out.len(), 7);
        for (i, (_, a)) in out.iter().enumerate() {
            assert!(a.2 > 0 && a.3 > 0);
            for (_, b) in &out[i + 1..] {
                let overlap = a.0 < b.0 + b.2 && b.0 < a.0 + a.2 && a.1 < b.1 + b.3 && b.1 < a.1 + a.3;
                assert!(!overlap, "{a:?} overlaps {b:?}");
            }
        }
    }
}
