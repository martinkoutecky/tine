use super::WholeGraph;

impl WholeGraph {
    /// All template names, including duplicates, in discovery order. O(T + name
    /// bytes), no page reads or subtree copies. Uses this view's publication.
    pub fn template_names(&self) -> Vec<String> {
        self.graph.template_names()
    }
}
