// ViewModel DTOs and mappers skeleton
pub struct TrackRowVm {
    pub id: String,
    pub title: String,
}

impl TrackRowVm {
    pub fn new(id: impl Into<String>, title: impl Into<String>) -> Self {
        Self { id: id.into(), title: title.into() }
    }
}
