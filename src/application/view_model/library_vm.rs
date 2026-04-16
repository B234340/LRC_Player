use crate::application::dto::LibraryDto;

#[derive(Debug, Clone)]
pub struct LibraryVm {
    pub album_count: usize,
    pub track_count: usize,
}

impl From<&LibraryDto> for LibraryVm {
    fn from(dto: &LibraryDto) -> Self {
        Self {
            album_count: 0,
            track_count: dto.track_count,
        }
    }
}
