//! Pure fold model for `/traces` tree views.
//!
//! A `FoldTree` is a preorder list of nodes; a `FoldState` records which
//! foldable nodes are collapsed, keyed by a stable `K` so state survives
//! live requeries. Leaves are never stored as collapsed.

use std::collections::HashSet;
use std::hash::Hash;
use std::ops::Range;

/// One node in preorder. `parent` is the index of its parent; `depth` is 0 for roots.
#[derive(Clone, Debug)]
pub(crate) struct FoldNode<K> {
    pub key: K,
    pub parent: Option<usize>,
    pub depth: usize,
}

/// Preorder node list. Invariant: `parent < child index` and `depth == parent.depth + 1`.
#[derive(Clone, Debug)]
pub(crate) struct FoldTree<K> {
    pub nodes: Vec<FoldNode<K>>,
}

/// Collapsed-node set keyed by stable identity.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct FoldState<K: Eq + Hash + Clone> {
    collapsed: HashSet<K>,
}

impl<K: Eq + Hash + Clone> FoldTree<K> {
    /// True when node `i` has at least one child (the node directly after it).
    pub fn has_children(&self, i: usize) -> bool {
        self.nodes.get(i + 1).is_some_and(|n| n.parent == Some(i))
    }

    /// Index range of node `i` and all its descendants.
    pub fn subtree(&self, i: usize) -> Range<usize> {
        let depth = self.nodes.get(i).map_or(0, |n| n.depth);
        let end = self
            .nodes
            .iter()
            .enumerate()
            .skip(i + 1)
            .find(|(_, n)| n.depth <= depth)
            .map_or(self.nodes.len(), |(j, _)| j);
        i..end
    }

    /// Indices of nodes to render, in order. Descendants of collapsed nodes are skipped.
    pub fn visible(&self, st: &FoldState<K>) -> Vec<usize> {
        let mut out = Vec::with_capacity(self.nodes.len());
        let mut i = 0;
        while let Some(node) = self.nodes.get(i) {
            out.push(i);
            i = if self.has_children(i) && st.is_collapsed(&node.key) {
                self.subtree(i).end
            } else {
                i + 1
            };
        }
        out
    }
}

impl<K: Eq + Hash + Clone> FoldState<K> {
    /// Empty state: everything expanded.
    pub fn new() -> Self {
        Self {
            collapsed: HashSet::new(),
        }
    }

    /// State with exactly the given keys collapsed.
    pub fn with_collapsed<I: IntoIterator<Item = K>>(keys: I) -> Self {
        Self {
            collapsed: keys.into_iter().collect(),
        }
    }

    /// True when `key` is stored as collapsed.
    pub fn is_collapsed(&self, key: &K) -> bool {
        self.collapsed.contains(key)
    }

    /// Flip node `i`'s fold. No-op on leaves.
    pub fn toggle(&mut self, tree: &FoldTree<K>, i: usize) {
        if !tree.has_children(i) {
            return;
        }
        let Some(node) = tree.nodes.get(i) else {
            return;
        };
        if !self.collapsed.remove(&node.key) {
            self.collapsed.insert(node.key.clone());
        }
    }

    /// Collapse node `i`. If `i` is a leaf or already collapsed, returns its parent index
    /// so the caller can move selection there; otherwise returns `None`.
    pub fn collapse(&mut self, tree: &FoldTree<K>, i: usize) -> Option<usize> {
        let node = tree.nodes.get(i)?;
        if !tree.has_children(i) || self.is_collapsed(&node.key) {
            return node.parent;
        }
        self.collapsed.insert(node.key.clone());
        None
    }

    /// Expand node `i` only. No-op on leaves.
    pub fn expand(&mut self, tree: &FoldTree<K>, i: usize) {
        if let Some(node) = tree.nodes.get(i) {
            self.collapsed.remove(&node.key);
        }
    }

    /// Expand node `i` and every descendant. Nodes outside the subtree are untouched.
    pub fn expand_subtree(&mut self, tree: &FoldTree<K>, i: usize) {
        for j in tree.subtree(i) {
            self.expand(tree, j);
        }
    }

    /// Collapse node `i` and every descendant that has children. Nodes outside the subtree are untouched.
    pub fn collapse_subtree(&mut self, tree: &FoldTree<K>, i: usize) {
        for j in tree.subtree(i) {
            if tree.has_children(j) {
                self.collapse(tree, j);
            }
        }
    }

    /// Smart toggle over the whole tree: if any foldable node is collapsed, expand all;
    /// otherwise collapse all foldable nodes.
    pub fn toggle_all(&mut self, tree: &FoldTree<K>) {
        let any_collapsed = tree
            .nodes
            .iter()
            .enumerate()
            .any(|(j, n)| tree.has_children(j) && self.is_collapsed(&n.key));
        if any_collapsed {
            self.collapsed.clear();
        } else {
            for j in 0..tree.nodes.len() {
                self.collapse(tree, j);
            }
        }
    }

    /// Drop stored keys that no longer appear in `tree`.
    pub fn retain(&mut self, tree: &FoldTree<K>) {
        let live: HashSet<&K> = tree.nodes.iter().map(|n| &n.key).collect();
        self.collapsed.retain(|k| live.contains(k));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a tree from `(key, depth)` pairs in preorder; parents are derived from depth.
    fn tree(spec: &[(&'static str, usize)]) -> FoldTree<&'static str> {
        let mut stack: Vec<usize> = Vec::new();
        let mut nodes = Vec::new();
        for (i, &(key, depth)) in spec.iter().enumerate() {
            stack.truncate(depth);
            nodes.push(FoldNode {
                key,
                parent: stack.last().copied(),
                depth,
            });
            stack.push(i);
        }
        FoldTree { nodes }
    }

    /// 0 a(d0) / 1 a1(d1) / 2 a1x(d2) / 3 a1x1(d3) / 4 a1y(d2) / 5 a2(d1) / 6 b(d0) / 7 b1(d1)
    fn sample() -> FoldTree<&'static str> {
        tree(&[
            ("a", 0),
            ("a1", 1),
            ("a1x", 2),
            ("a1x1", 3),
            ("a1y", 2),
            ("a2", 1),
            ("b", 0),
            ("b1", 1),
        ])
    }

    fn all() -> Vec<usize> {
        (0..8).collect()
    }

    #[test]
    fn has_children_detects_only_foldable_nodes() {
        let t = sample();
        let foldable: Vec<bool> = (0..8).map(|i| t.has_children(i)).collect();
        assert_eq!(
            foldable,
            vec![true, true, true, false, false, false, true, false]
        );
    }

    #[test]
    fn parents_are_preorder_and_depth_consistent() {
        let t = sample();
        for (i, n) in t.nodes.iter().enumerate() {
            match n.parent {
                None => assert_eq!(n.depth, 0),
                Some(p) => {
                    assert!(p < i);
                    assert_eq!(t.nodes.get(p).map(|pn| pn.depth + 1), Some(n.depth));
                }
            }
        }
    }

    #[test]
    fn subtree_ranges() {
        let t = sample();
        assert_eq!(t.subtree(0), 0..6);
        assert_eq!(t.subtree(1), 1..5);
        assert_eq!(t.subtree(2), 2..4);
        assert_eq!(t.subtree(3), 3..4);
        assert_eq!(t.subtree(5), 5..6);
        assert_eq!(t.subtree(6), 6..8);
        assert_eq!(t.subtree(7), 7..8);
    }

    #[test]
    fn subtree_of_last_root_runs_to_end() {
        let t = tree(&[("x", 0), ("x1", 1), ("x1a", 2)]);
        assert_eq!(t.subtree(0), 0..3);
        assert_eq!(t.subtree(1), 1..3);
    }

    #[test]
    fn visible_all_expanded_is_everything() {
        let t = sample();
        assert_eq!(t.visible(&FoldState::new()), all());
    }

    #[test]
    fn visible_skips_children_of_collapsed_root() {
        let t = sample();
        let st = FoldState::with_collapsed(["a"]);
        assert_eq!(t.visible(&st), vec![0, 6, 7]);
    }

    #[test]
    fn visible_skips_collapsed_mid_depth_only() {
        let t = sample();
        let st = FoldState::with_collapsed(["a1"]);
        assert_eq!(t.visible(&st), vec![0, 1, 5, 6, 7]);
    }

    #[test]
    fn visible_skips_deepest_collapsed_only() {
        let t = sample();
        let st = FoldState::with_collapsed(["a1x"]);
        assert_eq!(t.visible(&st), vec![0, 1, 2, 5, 6, 7]);
    }

    #[test]
    fn visible_nested_collapse_under_collapsed_ancestor() {
        let t = sample();
        let st = FoldState::with_collapsed(["a", "a1x"]);
        assert_eq!(t.visible(&st), vec![0, 6, 7]);
    }

    #[test]
    fn visible_ignores_stale_keys() {
        let t = sample();
        let st = FoldState::with_collapsed(["gone"]);
        assert_eq!(t.visible(&st), all());
    }

    #[test]
    fn toggle_leaf_is_noop() {
        let t = sample();
        let mut st = FoldState::new();
        st.toggle(&t, 3);
        assert_eq!(st, FoldState::new());
        let mut st = FoldState::with_collapsed(["a"]);
        st.toggle(&t, 3);
        assert_eq!(st, FoldState::with_collapsed(["a"]));
    }

    #[test]
    fn toggle_flips_foldable_node() {
        let t = sample();
        let mut st = FoldState::new();
        st.toggle(&t, 1);
        assert!(st.is_collapsed(&"a1"));
        assert_eq!(t.visible(&st), vec![0, 1, 5, 6, 7]);
        st.toggle(&t, 1);
        assert!(!st.is_collapsed(&"a1"));
        assert_eq!(t.visible(&st), all());
    }

    #[test]
    fn collapse_expanded_parent_returns_none_and_collapses() {
        let t = sample();
        let mut st = FoldState::new();
        assert_eq!(st.collapse(&t, 2), None);
        assert!(st.is_collapsed(&"a1x"));
        assert_eq!(t.visible(&st), vec![0, 1, 2, 5, 6, 7]);
    }

    #[test]
    fn collapse_leaf_returns_parent() {
        let t = sample();
        let mut st = FoldState::new();
        assert_eq!(st.collapse(&t, 3), Some(2));
        assert_eq!(st, FoldState::new());
    }

    #[test]
    fn collapse_already_collapsed_returns_parent() {
        let t = sample();
        let mut st = FoldState::with_collapsed(["a1x"]);
        assert_eq!(st.collapse(&t, 2), Some(1));
        assert!(st.is_collapsed(&"a1x"));
    }

    #[test]
    fn collapse_root_returns_none() {
        let t = sample();
        let mut st = FoldState::with_collapsed(["a"]);
        assert_eq!(st.collapse(&t, 0), None);
        let leaf_root = tree(&[("solo", 0)]);
        assert_eq!(FoldState::new().collapse(&leaf_root, 0), None);
    }

    #[test]
    fn collapse_out_of_range_is_none() {
        let t = sample();
        assert_eq!(FoldState::new().collapse(&t, 99), None);
    }

    #[test]
    fn expand_only_affects_node() {
        let t = sample();
        let mut st = FoldState::with_collapsed(["a", "a1"]);
        st.expand(&t, 1);
        assert!(!st.is_collapsed(&"a1"));
        assert!(st.is_collapsed(&"a"));
    }

    #[test]
    fn expand_leaf_is_noop() {
        let t = sample();
        let mut st = FoldState::with_collapsed(["a"]);
        st.expand(&t, 3);
        assert_eq!(st, FoldState::with_collapsed(["a"]));
    }

    #[test]
    fn expand_subtree_only_touches_subtree() {
        let t = sample();
        let mut st = FoldState::with_collapsed(["a", "a1", "a1x", "b"]);
        st.expand_subtree(&t, 1);
        assert_eq!(st, FoldState::with_collapsed(["a", "b"]));
    }

    #[test]
    fn expand_subtree_from_root_clears_its_branch_only() {
        let t = sample();
        let mut st = FoldState::with_collapsed(["a", "a1", "a1x", "b"]);
        st.expand_subtree(&t, 0);
        assert_eq!(st, FoldState::with_collapsed(["b"]));
        assert_eq!(t.visible(&st), vec![0, 1, 2, 3, 4, 5, 6]);
    }

    #[test]
    fn collapse_subtree_only_touches_subtree() {
        let t = sample();
        let mut st = FoldState::with_collapsed(["b"]);
        st.collapse_subtree(&t, 1);
        assert_eq!(st, FoldState::with_collapsed(["b", "a1", "a1x"]));
        assert!(!st.is_collapsed(&"a"));
        assert!(!st.is_collapsed(&"a2"));
    }

    #[test]
    fn collapse_subtree_from_root_collapses_nested_and_keeps_sibling_root() {
        let t = sample();
        let mut st = FoldState::new();
        st.collapse_subtree(&t, 0);
        assert_eq!(st, FoldState::with_collapsed(["a", "a1", "a1x"]));
        assert!(!st.is_collapsed(&"b"));
        assert_eq!(t.visible(&st), vec![0, 6, 7]);
    }

    #[test]
    fn collapse_subtree_on_leaf_is_noop() {
        let t = sample();
        let mut st = FoldState::new();
        st.collapse_subtree(&t, 3);
        assert_eq!(st, FoldState::new());
    }

    #[test]
    fn toggle_all_collapses_when_everything_expanded() {
        let t = sample();
        let mut st = FoldState::new();
        st.toggle_all(&t);
        assert_eq!(st, FoldState::with_collapsed(["a", "a1", "a1x", "b"]));
        assert_eq!(t.visible(&st), vec![0, 6]);
    }

    #[test]
    fn toggle_all_expands_when_any_collapsed() {
        let t = sample();
        let mut st = FoldState::with_collapsed(["a1"]);
        st.toggle_all(&t);
        assert_eq!(st, FoldState::new());
        assert_eq!(t.visible(&st), all());
    }

    #[test]
    fn toggle_all_expands_when_all_collapsed() {
        let t = sample();
        let mut st = FoldState::with_collapsed(["a", "a1", "a1x", "b"]);
        st.toggle_all(&t);
        assert_eq!(st, FoldState::new());
    }

    #[test]
    fn toggle_all_ignores_stale_keys_when_deciding() {
        let t = sample();
        let mut st = FoldState::with_collapsed(["gone"]);
        st.toggle_all(&t);
        assert_eq!(st, FoldState::with_collapsed(["a", "a1", "a1x", "b"]));
    }

    #[test]
    fn toggle_all_on_tree_without_foldables_is_noop() {
        let t = tree(&[("x", 0), ("y", 0)]);
        let mut st = FoldState::new();
        st.toggle_all(&t);
        assert_eq!(st, FoldState::new());
        let mut st = FoldState::with_collapsed(["gone"]);
        st.toggle_all(&t);
        assert_eq!(st, FoldState::with_collapsed(["gone"]));
    }

    #[test]
    fn toggle_all_on_empty_tree_is_noop() {
        let t: FoldTree<&'static str> = FoldTree { nodes: Vec::new() };
        let mut st = FoldState::new();
        st.toggle_all(&t);
        assert_eq!(st, FoldState::new());
        assert!(t.visible(&st).is_empty());
    }

    #[test]
    fn retain_drops_keys_not_in_tree() {
        let t = sample();
        let mut st = FoldState::with_collapsed(["a", "gone", "a1x", "also-gone"]);
        st.retain(&t);
        assert_eq!(st, FoldState::with_collapsed(["a", "a1x"]));
    }

    #[test]
    fn retain_keeps_keys_for_nodes_that_became_leaves() {
        let t = tree(&[("a", 0)]);
        let mut st = FoldState::with_collapsed(["a"]);
        st.retain(&t);
        assert_eq!(st, FoldState::with_collapsed(["a"]));
    }

    #[test]
    fn retain_on_empty_tree_clears_state() {
        let t: FoldTree<&'static str> = FoldTree { nodes: Vec::new() };
        let mut st = FoldState::with_collapsed(["a"]);
        st.retain(&t);
        assert_eq!(st, FoldState::new());
    }
}
