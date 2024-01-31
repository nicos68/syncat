use crate::line::Line;
use crate::Config;

pub fn squeeze_blank_lines(config: &Config, source: Vec<Line>) -> Vec<Line> {
    if config.squeeze {
        source
            .into_iter()
            .scan(false, |was_blank, line| {
                if line.is_empty() {
                    let output = if *was_blank { None } else { Some(line) };
                    *was_blank = true;
                    Some(output)
                } else {
                    *was_blank = false;
                    Some(Some(line))
                }
            })
            .flatten()
            .collect()
    } else {
        source
    }
}
