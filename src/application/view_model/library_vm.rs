use crate::application::dto::{LibraryDirectoryDto, LibraryDto};
use crate::application::view_model::{AlbumVm, ArtistVm, TrackRowVm};

#[derive(Debug, Clone)]
pub struct LibraryDirectoryVm {
    pub path: String,
    pub children: Vec<LibraryDirectoryVm>,
}

#[derive(Debug, Clone)]
pub struct LibraryVm {
    pub album_count: usize,
    pub track_count: usize,
    pub recent_locations: Vec<String>,
    pub recent_directories: Vec<String>,
    pub directory_tree: Vec<LibraryDirectoryVm>,
    pub favorite_directories: Vec<String>,
    pub albums: Vec<AlbumVm>,
    pub selected_album: Option<String>,
    pub selected_album_tracks: Vec<TrackRowVm>,
    pub artists: Vec<ArtistVm>,
    pub selected_artist: Option<String>,
    pub selected_artist_tracks: Vec<TrackRowVm>,
    pub selected_directory: Option<String>,
    pub selected_directory_tracks: Vec<TrackRowVm>,
}

impl From<&LibraryDirectoryDto> for LibraryDirectoryVm {
    fn from(dto: &LibraryDirectoryDto) -> Self {
        Self {
            path: dto.path.clone(),
            children: dto.children.iter().map(LibraryDirectoryVm::from).collect(),
        }
    }
}

impl From<&LibraryDto> for LibraryVm {
    fn from(dto: &LibraryDto) -> Self {
        Self {
            album_count: dto.album_count,
            track_count: dto.track_count,
            recent_locations: dto.recent_locations.clone(),
            recent_directories: dto.recent_directories.clone(),
            directory_tree: dto
                .directory_tree
                .iter()
                .map(LibraryDirectoryVm::from)
                .collect(),
            favorite_directories: dto.favorite_directories.clone(),
            albums: dto
                .albums
                .iter()
                .map(|title| AlbumVm {
                    id: title.clone(),
                    title: title.clone(),
                })
                .collect(),
            selected_album: dto.selected_album.clone(),
            selected_album_tracks: dto
                .selected_album_tracks
                .iter()
                .map(|title| TrackRowVm {
                    id: title.clone(),
                    title: title.clone(),
                })
                .collect(),
            artists: dto
                .artists
                .iter()
                .map(|name| ArtistVm {
                    id: name.clone(),
                    name: name.clone(),
                })
                .collect(),
            selected_artist: dto.selected_artist.clone(),
            selected_artist_tracks: dto
                .selected_artist_tracks
                .iter()
                .map(|title| TrackRowVm {
                    id: title.clone(),
                    title: title.clone(),
                })
                .collect(),
            selected_directory: dto.selected_directory.clone(),
            selected_directory_tracks: dto
                .selected_directory_tracks
                .iter()
                .map(|title| TrackRowVm {
                    id: title.clone(),
                    title: title.clone(),
                })
                .collect(),
        }
    }
}
