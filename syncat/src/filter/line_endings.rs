use crate::line::Line;
use crate::Config;

pub fn line_endings(
    config: &Config,
    source: Vec<Line>,
) -> Vec<Line> {
    if config.show_line_endings {
        source.into_iter().map(Line::with_line_ending).collect()
    } else {
        source
    }
}
