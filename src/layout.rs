// =============================================================================
// Tiling layout tree
//
// The workspace is a binary tree of splits whose leaves each hold one panel id
// (an index into the App's panel arena). This is what turns the old fixed
// dual-pane view into an arbitrarily-tileable one, like a tiling window manager.
//
//   * Dir::Horizontal lays its two children out side by side (a vertical seam).
//   * Dir::Vertical stacks them top/bottom (a horizontal seam).
//   * `ratio` is the fraction of space given to the first child (clamped).
//
// The module is pure geometry/structure — it owns no panels and does no IO, so
// it is fully unit-testable.
// =============================================================================

use ratatui::layout::Rect;

pub type PanelId = usize;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Dir {
    Horizontal, // children left | right
    Vertical,   // children top / bottom
}

#[derive(Clone, Debug)]
pub enum Node {
    Leaf(PanelId),
    Split {
        dir: Dir,
        ratio: f32,
        first: Box<Node>,
        second: Box<Node>,
    },
}

/// A resize seam between two children, with the path to its Split node so the
/// caller can adjust that node's ratio.
pub struct Divider {
    pub rect: Rect,    // the 1px seam, for hit-testing
    pub area: Rect,    // the split's full area, for computing the dragged ratio
    pub dir: Dir,
    pub path: Vec<u8>, // 0 = first child, 1 = second child
}

const MIN_RATIO: f32 = 0.1;
const MAX_RATIO: f32 = 0.9;

impl Node {
    pub fn leaf(id: PanelId) -> Node {
        Node::Leaf(id)
    }

    /// All leaf panel ids in left-to-right / top-to-bottom order.
    pub fn leaves(&self) -> Vec<PanelId> {
        let mut out = Vec::new();
        self.collect_leaves(&mut out);
        out
    }

    fn collect_leaves(&self, out: &mut Vec<PanelId>) {
        match self {
            Node::Leaf(id) => out.push(*id),
            Node::Split { first, second, .. } => {
                first.collect_leaves(out);
                second.collect_leaves(out);
            }
        }
    }

    pub fn first_leaf(&self) -> PanelId {
        match self {
            Node::Leaf(id) => *id,
            Node::Split { first, .. } => first.first_leaf(),
        }
    }

    pub fn contains(&self, id: PanelId) -> bool {
        match self {
            Node::Leaf(l) => *l == id,
            Node::Split { first, second, .. } => first.contains(id) || second.contains(id),
        }
    }

    /// Split the leaf holding `target` into two, with `new_id` placed in the
    /// second child. No-op if `target` is not present.
    pub fn split_leaf(&mut self, target: PanelId, new_id: PanelId, dir: Dir) {
        match self {
            Node::Leaf(id) if *id == target => {
                let existing = *id;
                *self = Node::Split {
                    dir,
                    ratio: 0.5,
                    first: Box::new(Node::Leaf(existing)),
                    second: Box::new(Node::Leaf(new_id)),
                };
            }
            Node::Leaf(_) => {}
            Node::Split { first, second, .. } => {
                first.split_leaf(target, new_id, dir);
                second.split_leaf(target, new_id, dir);
            }
        }
    }

    /// Remove the leaf holding `target`, collapsing its parent split into the
    /// surviving sibling. Returns the new root, or None if removing it would
    /// leave the tree empty (caller should refuse to close the last pane).
    pub fn close_leaf(self, target: PanelId) -> Option<Node> {
        match self {
            Node::Leaf(id) => {
                if id == target {
                    None
                } else {
                    Some(Node::Leaf(id))
                }
            }
            Node::Split {
                dir,
                ratio,
                first,
                second,
            } => {
                // If a direct child is the target leaf, collapse to the sibling.
                if matches!(*first, Node::Leaf(id) if id == target) {
                    return Some(*second);
                }
                if matches!(*second, Node::Leaf(id) if id == target) {
                    return Some(*first);
                }
                // Otherwise recurse; a child split may itself become a leaf.
                let new_first = first.close_leaf(target);
                let new_second = second.close_leaf(target);
                match (new_first, new_second) {
                    (Some(f), Some(s)) => Some(Node::Split {
                        dir,
                        ratio,
                        first: Box::new(f),
                        second: Box::new(s),
                    }),
                    (Some(only), None) | (None, Some(only)) => Some(only),
                    (None, None) => None,
                }
            }
        }
    }

    /// Adjust the ratio of the Split node at `path`.
    pub fn set_ratio(&mut self, path: &[u8], ratio: f32) {
        match self {
            Node::Split {
                ratio: r,
                first,
                second,
                ..
            } => {
                if path.is_empty() {
                    *r = ratio.clamp(MIN_RATIO, MAX_RATIO);
                } else if path[0] == 0 {
                    first.set_ratio(&path[1..], ratio);
                } else {
                    second.set_ratio(&path[1..], ratio);
                }
            }
            Node::Leaf(_) => {}
        }
    }

    pub fn get_ratio(&self, path: &[u8]) -> Option<f32> {
        match self {
            Node::Split {
                ratio,
                first,
                second,
                ..
            } => {
                if path.is_empty() {
                    Some(*ratio)
                } else if path[0] == 0 {
                    first.get_ratio(&path[1..])
                } else {
                    second.get_ratio(&path[1..])
                }
            }
            Node::Leaf(_) => None,
        }
    }

    /// Path to the closest ancestor Split of `target` whose direction is `dir`
    /// (used by keyboard resize: "make the focused pane wider/taller").
    pub fn ancestor_split(&self, target: PanelId, want: Dir) -> Option<(Vec<u8>, bool)> {
        // Returns (path_to_split, target_is_in_first_child).
        self.ancestor_inner(target, want, &mut Vec::new())
    }

    fn ancestor_inner(&self, target: PanelId, want: Dir, path: &mut Vec<u8>) -> Option<(Vec<u8>, bool)> {
        if let Node::Split {
            dir,
            first,
            second,
            ..
        } = self
        {
            // Prefer the deepest matching split, so descend first.
            path.push(0);
            if let Some(found) = first.ancestor_inner(target, want, path) {
                return Some(found);
            }
            path.pop();
            path.push(1);
            if let Some(found) = second.ancestor_inner(target, want, path) {
                return Some(found);
            }
            path.pop();

            if *dir == want {
                if first.contains(target) {
                    return Some((path.clone(), true));
                }
                if second.contains(target) {
                    return Some((path.clone(), false));
                }
            }
        }
        None
    }

    /// Compute the screen rectangle for every leaf.
    pub fn rects(&self, area: Rect) -> Vec<(PanelId, Rect)> {
        let mut out = Vec::new();
        self.rects_inner(area, &mut out);
        out
    }

    fn rects_inner(&self, area: Rect, out: &mut Vec<(PanelId, Rect)>) {
        match self {
            Node::Leaf(id) => out.push((*id, area)),
            Node::Split {
                dir,
                ratio,
                first,
                second,
            } => {
                let (a, b) = split_rect(area, *dir, *ratio);
                first.rects_inner(a, out);
                second.rects_inner(b, out);
            }
        }
    }

    /// Compute the resize seams (1px lines on the boundary between children).
    pub fn dividers(&self, area: Rect) -> Vec<Divider> {
        let mut out = Vec::new();
        self.dividers_inner(area, &mut Vec::new(), &mut out);
        out
    }

    fn dividers_inner(&self, area: Rect, path: &mut Vec<u8>, out: &mut Vec<Divider>) {
        if let Node::Split {
            dir,
            ratio,
            first,
            second,
        } = self
        {
            let (a, b) = split_rect(area, *dir, *ratio);
            // The shared seam sits on the overlapped column/row.
            let seam = match dir {
                Dir::Horizontal => Rect {
                    x: a.x + a.width - 1,
                    y: area.y,
                    width: 1,
                    height: area.height,
                },
                Dir::Vertical => Rect {
                    x: area.x,
                    y: a.y + a.height - 1,
                    width: area.width,
                    height: 1,
                },
            };
            out.push(Divider {
                rect: seam,
                area,
                dir: *dir,
                path: path.clone(),
            });
            path.push(0);
            first.dividers_inner(a, path, out);
            path.pop();
            path.push(1);
            second.dividers_inner(b, path, out);
            path.pop();
        }
    }
}

fn split_rect(area: Rect, dir: Dir, ratio: f32) -> (Rect, Rect) {
    let r = ratio.clamp(MIN_RATIO, MAX_RATIO);
    // The second child overlaps the first by one cell at the seam, so the two
    // bordered panels share a single divider line instead of drawing ║║ / ══.
    match dir {
        Dir::Horizontal => {
            let w = ((area.width as f32) * r).round() as u16;
            let w = w.clamp(2, area.width.saturating_sub(1).max(2));
            let a = Rect { x: area.x, y: area.y, width: w, height: area.height };
            let b = Rect {
                x: area.x + w - 1,
                y: area.y,
                width: area.width - (w - 1),
                height: area.height,
            };
            (a, b)
        }
        Dir::Vertical => {
            let h = ((area.height as f32) * r).round() as u16;
            let h = h.clamp(2, area.height.saturating_sub(1).max(2));
            let a = Rect { x: area.x, y: area.y, width: area.width, height: h };
            let b = Rect {
                x: area.x,
                y: area.y + h - 1,
                width: area.width,
                height: area.height - (h - 1),
            };
            (a, b)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_leaves_and_split() {
        let mut root = Node::leaf(0);
        assert_eq!(root.leaves(), vec![0]);
        root.split_leaf(0, 1, Dir::Horizontal);
        assert_eq!(root.leaves(), vec![0, 1]);
        root.split_leaf(1, 2, Dir::Vertical);
        assert_eq!(root.leaves(), vec![0, 1, 2]);
        assert!(root.contains(2));
        assert!(!root.contains(9));
    }

    #[test]
    fn test_close_collapses_to_sibling() {
        let mut root = Node::leaf(0);
        root.split_leaf(0, 1, Dir::Horizontal);
        let root = root.close_leaf(1).unwrap();
        assert_eq!(root.leaves(), vec![0]);
        assert!(matches!(root, Node::Leaf(0)));
    }

    #[test]
    fn test_close_last_leaf_is_none() {
        let root = Node::leaf(0);
        assert!(root.close_leaf(0).is_none());
    }

    #[test]
    fn test_close_nested() {
        // (0 | (1 / 2)) -> close 1 -> (0 | 2)
        let mut root = Node::leaf(0);
        root.split_leaf(0, 1, Dir::Horizontal);
        root.split_leaf(1, 2, Dir::Vertical);
        let root = root.close_leaf(1).unwrap();
        assert_eq!(root.leaves(), vec![0, 2]);
    }

    #[test]
    fn test_rects_partition_area() {
        let mut root = Node::leaf(0);
        root.split_leaf(0, 1, Dir::Horizontal);
        let area = Rect::new(0, 0, 100, 40);
        let rects = root.rects(area);
        assert_eq!(rects.len(), 2);
        // Side by side, full height each; second overlaps the seam by one cell
        // so their shared border renders as a single line.
        let (_, a) = rects[0];
        let (_, b) = rects[1];
        assert_eq!(a.height, 40);
        assert_eq!(b.height, 40);
        assert_eq!(b.x, a.x + a.width - 1);
        assert_eq!(b.x + b.width, a.x + 100); // together still span the full width
    }

    #[test]
    fn test_set_and_get_ratio() {
        let mut root = Node::leaf(0);
        root.split_leaf(0, 1, Dir::Horizontal);
        root.set_ratio(&[], 0.7);
        assert!((root.get_ratio(&[]).unwrap() - 0.7).abs() < 1e-6);
        // clamp
        root.set_ratio(&[], 0.99);
        assert!(root.get_ratio(&[]).unwrap() <= MAX_RATIO + 1e-6);
    }

    #[test]
    fn test_dividers_count() {
        let mut root = Node::leaf(0);
        root.split_leaf(0, 1, Dir::Horizontal);
        root.split_leaf(1, 2, Dir::Vertical);
        // two splits -> two dividers
        assert_eq!(root.dividers(Rect::new(0, 0, 100, 40)).len(), 2);
    }

    #[test]
    fn test_ancestor_split() {
        let mut root = Node::leaf(0);
        root.split_leaf(0, 1, Dir::Horizontal); // root split is Horizontal
        let (path, in_first) = root.ancestor_split(0, Dir::Horizontal).unwrap();
        assert_eq!(path, Vec::<u8>::new());
        assert!(in_first);
        let (_p, in_first2) = root.ancestor_split(1, Dir::Horizontal).unwrap();
        assert!(!in_first2);
        // no vertical ancestor exists
        assert!(root.ancestor_split(0, Dir::Vertical).is_none());
    }
}
