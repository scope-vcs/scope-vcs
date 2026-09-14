use super::{ReviewItem, ReviewState};
use scope_domain::dependency_analysis::StoredDependencyAnalysis;

impl ReviewState {
    pub fn complete_dependency_analysis(
        &mut self,
        result: Result<StoredDependencyAnalysis, String>,
    ) {
        self.dependencies.complete(result, &self.config);
        self.rebuild_visible_items();
    }

    pub(super) fn collect_dependency_items(&self, items: &mut Vec<ReviewItem>) {
        if !self.dependencies.is_visible() {
            return;
        }
        items.push(ReviewItem::DependencySummary);
        let Some(report) = self
            .dependencies
            .report()
            .filter(|_| self.dependencies.expanded())
        else {
            return;
        };
        items.extend((0..report.findings.len()).map(ReviewItem::DependencyFinding));
        items.extend((0..report.gaps.len()).map(ReviewItem::DependencyGap));
        items.push(ReviewItem::DependencyCoverage);
    }

    pub(super) fn jump_to_path(&mut self, path: &str) {
        let normalized = format!("/{}", path.trim_start_matches('/'));
        let Some(id) = self
            .tree
            .nodes()
            .iter()
            .find(|node| node.path == normalized)
            .map(|node| node.id)
        else {
            self.message = format!("Path is not present in the reviewed commit: {path}");
            return;
        };
        let mut parent = self.tree.node(id).parent;
        while let Some(parent_id) = parent {
            self.expanded_tree_nodes.insert(parent_id);
            parent = self.tree.node(parent_id).parent;
        }
        self.rebuild_visible_items();
        self.move_cursor_to_item(ReviewItem::TreeNode(id));
        self.message = format!("Selected {path}");
    }
}
