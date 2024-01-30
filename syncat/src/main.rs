use clap_serde_derive::clap::{ArgAction, Parser};
use clap_serde_derive::serde::Deserialize;
use clap_serde_derive::ClapSerde;
use std::fs;
use std::fs::File;
use std::io::{self, Read};
use std::path::{Path, PathBuf};

mod colorize;
mod config;
mod dirs;
mod error;
mod filter;
mod language;
mod line;
mod meta_stylesheet;
mod package_manager;

use colorize::Colorizer;
use error::{Error, Result};
use language::LangMap;
use line::Line;
use meta_stylesheet::MetaStylesheet;

/// Syntax aware cat utility.
#[derive(Parser)]
// ClapSerde’s proc macros must be kept separate
#[rustfmt::skip]
#[derive(Debug)]
#[clap(name = "syncat")]
#[clap(rename_all = "kebab-case")]
pub struct Opts {
    /// Use Git to show recent changes
    #[arg(short, long)]
    git: bool,

    /// Squeeze consecutive blank lines into one
    #[arg(short, long)]
    squeeze: bool,

    /// Show line endings
    #[arg(short = 'e', long = "endings")]
    show_line_endings: bool,

    /// Number non-empty input lines (overrides -n)
    #[arg(short = 'b', long)]
    numbered_nonblank: bool,

    /// Number all input lines
    #[arg(short, long)]
    numbered: bool,

    /// Prints a parsed s-expression, for debugging and theme creation
    #[arg(long)]
    dev: bool,

    /// The language to use to parse the files
    #[arg(short, long)]
    language: Option<String>,

    /// Soft-wrap lines at a fixed width
    #[arg(short, long)]
    wrap: Option<usize>,

    /// Files to parse and print
    #[arg(name = "FILE")]
    files: Vec<PathBuf>,

    #[command(subcommand)]
    command: Option<Subcommand>,

    #[command(flatten)]
    config: <Config as ClapSerde>::Opt,
}

#[derive(ClapSerde, Deserialize)]
// ClapSerde’s proc macros must be kept separate
#[rustfmt::skip]
#[derive(Debug)]
pub struct Config {
    /// Level of framing around each file. Repeat for bigger frame
    #[arg(short, long, action=ArgAction::Count)]
    frame: u8,
}

#[derive(Parser, Debug)]
enum Subcommand {
    /// Initialize the config directory by filling it with the default configuration.
    ///
    /// If the config directory already exists, it will not be created. An alternative path
    /// may be specified.
    Init {
        #[arg(short, long)]
        out: Option<PathBuf>,
    },
    /// Installs all languages listed in the `languages.toml` file. Previously installed packages
    /// will be updated, if updates are available. This process may take a long time, depending on
    /// how many languages are being installed.
    Install {
        /// If you provide a list of languages, only only those languages will be installed now. They must
        /// be listed in the `languages.toml` file first.
        languages: Vec<String>,
    },
    /// Remove an installed language.
    Remove {
        /// The name of the language to remove.
        language: String,
    },
    /// List all installed languages.
    List,
}

struct Source<'a> {
    language: Option<String>,
    source: String,
    path: Option<&'a Path>,
}

/// The syncat instance holds globally loaded configuration to prevent loading
/// it twice.
struct Syncat {
    opts: Opts,
    meta_style: MetaStylesheet,
    lang_map: LangMap,
    config: Config,
}

impl Syncat {
    fn new(opts: Opts, config: Config) -> error::Result<Self> {
        let lang_map = LangMap::open()?;
        let meta_style = MetaStylesheet::from_file()?;
        Ok(Self {
            opts,
            lang_map,
            meta_style,
            config,
        })
    }

    fn colorize(&self, language: Option<&str>, source: String) -> crate::Result<String> {
        let language = self
            .opts
            .language
            .as_deref()
            .or(language)
            .and_then(|language| self.lang_map.get(language));
        let Some(language) = language else {
            // Language unknown, so just print
            return Ok(source);
        };
        let Some(mut parser) = language.parser()? else {
            // Language not installed, so also just print
            return Ok(source);
        };
        let tree = parser.parse(&source, None).unwrap();
        let colorizer = Colorizer {
            source: source.as_str(),
            tree,
            stylesheet: language.style()?,
            lang_map: &self.lang_map,
        };
        if self.opts.dev {
            Ok(format!("{:?}", colorizer))
        } else {
            Ok(format!("{}", colorizer))
        }
    }

    fn transform(
        &self,
        language: Option<&str>,
        source: String,
        path: Option<&Path>,
    ) -> crate::Result<Vec<Line>> {
        let source = self.colorize(language, source)?;

        if self.opts.dev {
            Ok(vec![Line::new(source)])
        } else {
            let mut lines = source
                .lines()
                .map(|line| Line::new(line.to_owned()))
                .collect::<Vec<_>>();

            if !source.ends_with('\n') {
                if let Some(line) = lines.last_mut() {
                    line.no_newline = true;
                }
            }

            let lines = filter::git(&self.opts, lines, path);
            let lines = filter::squeeze_blank_lines(&self.opts, lines);
            let lines = filter::line_endings(&self.opts, lines);

            Ok(lines)
        }
    }

    fn print<'a>(
        &self,
        sources: impl IntoIterator<Item = Result<Source<'a>>> + ExactSizeIterator,
    ) -> crate::Result<()> {
        let count = sources.len();
        let mut line_numbers = filter::line_numbers(&self.opts, &self.config);
        for (index, source) in sources.into_iter().enumerate() {
            let Source {
                language,
                source,
                path,
            } = match source {
                Ok(source) => source,
                Err(error) => {
                    eprintln!("{error}");
                    continue;
                }
            };

            match self.transform(language.as_deref(), source, path) {
                Ok(lines) => {
                    let lines = line_numbers(lines);
                    // NOTE: frame is a bit weird, idk why it needs to move in and return the lines...
                    let lines = filter::frame_header(
                        (index, count),
                        &self.opts,
                        &self.config,
                        lines,
                        path,
                        &self.meta_style,
                    );
                    for line in &lines {
                        print!("{}", line.to_string(&self.meta_style, self.opts.wrap));
                    }
                    let _ = filter::frame_footer(
                        (index, count),
                        &self.opts,
                        &self.config,
                        lines,
                        path,
                        &self.meta_style,
                    );
                }
                Err(error) => match path {
                    Some(path) => eprintln!("syncat: {}: {error}", path.display()),
                    None => eprintln!("syncat: {error}"),
                },
            }
        }
        Ok(())
    }
}

fn try_main() -> error::Result<()> {
    let mut opts = Opts::parse();
    let config = if let Ok(mut config_file) = File::open(dirs::config().join("syncat.toml")) {
        let mut config_contents = String::new();
        config_file.read_to_string(&mut config_contents).unwrap();
        let config_from_file =
            Config::from(toml::from_str::<<Config as ClapSerde>::Opt>(&config_contents).unwrap());
        workaround_for_clap_serde(&mut opts, config_from_file)
    } else {
        Config::from(&mut opts.config)
    };
    match &opts.command {
        Some(subcommand) => package_manager::main(subcommand),
        None if opts.files.is_empty() && opts.language.is_none() => {
            // Mimic the behaviour of standard cat, printing lines as they come.
            // These lines cannot be syntax highlighted, as we do not know what the language is.
            loop {
                let mut line = String::new();
                if io::stdin()
                    .read_line(&mut line)
                    .map_err(|er| crate::Error::new("could not read stdin").with_source(er))?
                    == 0
                {
                    return Ok(());
                }
                print!("{}", line);
            }
        }
        None if opts.files.is_empty() => {
            // If a language is specified, read in the whole file and then attempt to print that
            // at once using the specified language.
            let mut stdin = io::stdin();
            let mut source = String::new();
            stdin
                .read_to_string(&mut source)
                .map_err(|er| crate::Error::new("could not read stdin").with_source(er))?;
            let syncat = Syncat::new(opts, config)?;
            syncat.print(std::iter::once(Ok(Source {
                language: None,
                source,
                path: None,
            })))
        }
        None => {
            // Attempt to style each of the supplied files, detecting languages based on extension
            // while respecting the override provided.
            //
            // TODO: Add detection for hashbang/vim modeline/etc.
            let files = opts.files.clone();
            let sources = files.iter().map(|path| {
                Ok(Source {
                    language: path
                        .extension()
                        .and_then(|s| s.to_str())
                        .map(|s| s.to_owned()),
                    source: fs::read_to_string(path).map_err(|err| {
                        Error::new("failed to read file")
                            .with_source(err)
                            .with_path(path)
                    })?,
                    path: Some(path.as_ref()),
                })
            });
            let syncat = Syncat::new(opts, config)?;
            syncat.print(sources)
        }
    }
}

fn workaround_for_clap_serde(opts: &mut Opts, config_from_file: Config) -> Config {
    let frame_value = if opts.config.frame != Some(0) {
        opts.config.frame.unwrap_or(config_from_file.frame)
    } else {
        config_from_file.frame
    };
    let mut config = config_from_file.merge(&mut opts.config);
    config.frame = frame_value;
    config
}

fn main() {
    if let Err(error) = try_main() {
        eprintln!("{}", error);
        std::process::exit(1);
    }
}
