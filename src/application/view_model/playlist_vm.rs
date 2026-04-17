use crate::application::dto::PlaylistDto;

#[derive(Debug, Clone)]
pub struct PlaylistVm {
    pub id: String,
    pub name: String,
    pub tracks: Vec<String>,
}

impl From<&PlaylistDto> for PlaylistVm {
    fn from(dto: &PlaylistDto) -> Self {
        Self {
            id: dto.id.clone(),
            name: dto.name.clone(),
            tracks: dto.tracks.clone(),
        }
    }
}
