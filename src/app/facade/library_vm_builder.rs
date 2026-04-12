// Library VM builder: extracted from `AppFacade` to reduce facade size.
use std::sync::Arc;

use crate::app::view_models::{DirectoryEntryVm, LibraryTabVm};

pub fn build_library_tab(facade: &super::AppFacade) -> LibraryTabVm {
    let nodes = facade
        .library
        .directories
        .nodes
        .get(facade.library.directories.root.0 as usize)
        .map(|root| {
            root.children
                .iter()
                .filter_map(|node_id| {
                    facade
                        .library
                        .directories
                        .nodes
                        .get(node_id.0 as usize)
                        .map(|node| DirectoryEntryVm {
                            node_id: *node_id,
                            name: Arc::from(
                                facade
                                    .strings
                                    .resolve(node.name)
                                    .unwrap_or_else(|| "Unknown".to_string())
                                    .into_boxed_str(),
                            ),
                            child_count: node.children.len(),
                            track_count: node.tracks.len(),
                        })
                })
                .collect()
        })
        .unwrap_or_default();

    let visible_tracks = facade
        .selected_directory
        .and_then(|node_id| facade.library.directories.nodes.get(node_id.0 as usize))
        .map(|node| facade.track_rows_for_tracks(&node.tracks))
        .unwrap_or_default();

    LibraryTabVm {
        nodes,
        selected_directory: facade.selected_directory,
        visible_tracks,
    }
}
