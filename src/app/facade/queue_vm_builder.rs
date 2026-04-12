use crate::app::view_models::QueueTabVm;

/// Build QueueTabVm from facade state.
pub fn build_queue_tab(facade: &super::AppFacade) -> QueueTabVm {
    QueueTabVm {
        rows: facade
            .queue
            .entries
            .iter()
            .enumerate()
            .map(|(index, entry)| facade.track_row_vm(entry.track_id, Some(index)))
            .collect(),
        repeat_mode: facade.queue.repeat_mode,
    }
}
