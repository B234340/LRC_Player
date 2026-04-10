use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ScanState {
    #[default]
    Idle,
    Scanning,
    Cancelling,
    Completed,
    Warning,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ScanProgress {
    pub discovered_files: u32,
    pub parsed_files: u32,
    pub skipped_files: u32,
    pub current_path: Option<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ScanConfig {
    pub include_dirs: Vec<PathBuf>,
    pub exclude_dirs: Vec<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScanModel {
    pub state: ScanState,
    pub last_visible_result: Option<ScanState>,
    pub progress: ScanProgress,
    pub config: ScanConfig,
}

impl Default for ScanModel {
    fn default() -> Self {
        Self {
            state: ScanState::Idle,
            last_visible_result: None,
            progress: ScanProgress::default(),
            config: ScanConfig::default(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScanCommand {
    StartFullScan,
    Stop,
    AddIncludeDir(PathBuf),
    RemoveIncludeDir(PathBuf),
    AddExcludeDir(PathBuf),
    RemoveExcludeDir(PathBuf),
}

impl ScanModel {
    pub fn apply(&mut self, command: ScanCommand) {
        match command {
            ScanCommand::StartFullScan => {
                self.state = ScanState::Scanning;
                self.progress = ScanProgress::default();
            }
            ScanCommand::Stop if self.state == ScanState::Scanning => {
                self.state = ScanState::Cancelling;
                self.last_visible_result = Some(ScanState::Cancelling);
            }
            ScanCommand::AddIncludeDir(path) => {
                if !self.config.include_dirs.contains(&path) {
                    self.config.include_dirs.push(path);
                }
            }
            ScanCommand::RemoveIncludeDir(path) => {
                self.config.include_dirs.retain(|existing| existing != &path);
            }
            ScanCommand::AddExcludeDir(path) => {
                if !self.config.exclude_dirs.contains(&path) {
                    self.config.exclude_dirs.push(path);
                }
            }
            ScanCommand::RemoveExcludeDir(path) => {
                self.config.exclude_dirs.retain(|existing| existing != &path);
            }
            _ => {}
        }
    }

    pub fn finish_with(&mut self, result: ScanState) {
        self.state = result;
        self.last_visible_result = Some(result);
    }

    pub fn visible_status(&self) -> ScanState {
        match self.state {
            ScanState::Idle => self.last_visible_result.unwrap_or(ScanState::Idle),
            other => other,
        }
    }

    pub fn reset_if_terminal(&mut self) {
        if matches!(
            self.state,
            ScanState::Cancelling | ScanState::Completed | ScanState::Warning | ScanState::Failed
        ) {
            self.state = ScanState::Idle;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{ScanCommand, ScanModel, ScanState};

    #[test]
    fn terminal_result_stays_visible_until_tab_change() {
        let mut scan = ScanModel::default();
        scan.apply(ScanCommand::StartFullScan);
        scan.finish_with(ScanState::Warning);

        assert_eq!(scan.visible_status(), ScanState::Warning);

        scan.reset_if_terminal();
        assert_eq!(scan.state, ScanState::Idle);
        assert_eq!(scan.visible_status(), ScanState::Warning);
    }
}