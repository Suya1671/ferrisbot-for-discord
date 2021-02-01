//! run rust code on the rust-lang playground

use crate::{api, commands::Args, Error};

use reqwest::header;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::str::FromStr;

#[derive(Debug, Serialize)]
struct PlaygroundRequest<'a> {
    channel: Channel,
    edition: Edition,
    code: &'a str,
    #[serde(rename = "crateType")]
    crate_type: CrateType,
    mode: Mode,
    tests: bool,
}

#[derive(Debug, Serialize)]
struct MiriRequest<'a> {
    edition: Edition,
    code: &'a str,
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
enum Channel {
    Stable,
    Beta,
    Nightly,
}

impl FromStr for Channel {
    type Err = Box<dyn std::error::Error>;

    fn from_str(s: &str) -> Result<Self, Error> {
        match s {
            "stable" => Ok(Channel::Stable),
            "beta" => Ok(Channel::Beta),
            "nightly" => Ok(Channel::Nightly),
            _ => Err(format!("invalid release channel `{}`", s).into()),
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize)]
enum Edition {
    #[serde(rename = "2015")]
    E2015,
    #[serde(rename = "2018")]
    E2018,
}

impl FromStr for Edition {
    type Err = Box<dyn std::error::Error>;

    fn from_str(s: &str) -> Result<Self, Error> {
        match s {
            "2015" => Ok(Edition::E2015),
            "2018" => Ok(Edition::E2018),
            _ => Err(format!("invalid edition `{}`", s).into()),
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize)]
enum CrateType {
    #[serde(rename = "bin")]
    Binary,
    #[serde(rename = "lib")]
    Library,
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
enum Mode {
    Debug,
    Release,
}

impl FromStr for Mode {
    type Err = Box<dyn std::error::Error>;

    fn from_str(s: &str) -> Result<Self, Error> {
        match s {
            "debug" => Ok(Mode::Debug),
            "release" => Ok(Mode::Release),
            _ => Err(format!("invalid compilation mode `{}`", s).into()),
        }
    }
}

#[derive(Debug, Deserialize)]
struct PlayResult {
    success: bool,
    stdout: String,
    stderr: String,
}

/// Returns a gist ID
fn post_gist(args: &Args, code: &str) -> Result<String, Error> {
    let mut payload = HashMap::new();
    payload.insert("code", code);

    let resp = args
        .http
        .post("https://play.rust-lang.org/meta/gist/")
        .header(header::REFERER, "https://discord.gg/rust-lang")
        .json(&payload)
        .send()?;

    let mut resp: HashMap<String, String> = resp.json()?;
    info!("gist response: {:?}", resp);

    let gist_id = resp.remove("id").ok_or("no gist found")?;
    Ok(gist_id)
}

fn url_from_gist(flags: &CommandFlags, gist_id: &str) -> String {
    format!(
        "https://play.rust-lang.org/?version={}&mode={}&edition={}&gist={}",
        match flags.channel {
            Channel::Nightly => "nightly",
            Channel::Beta => "beta",
            Channel::Stable => "stable",
        },
        match flags.mode {
            Mode::Debug => "debug",
            Mode::Release => "release",
        },
        match flags.edition {
            Edition::E2015 => "2015",
            Edition::E2018 => "2018",
        },
        gist_id
    )
}

struct CommandFlags {
    channel: Channel,
    mode: Mode,
    edition: Edition,
}

/// Returns the parsed flags, a bool whether stderr output is desired, and a String of parse errors
fn parse_flags(args: &Args) -> (CommandFlags, bool, String) {
    let mut errors = String::new();

    let mut flags = CommandFlags {
        channel: Channel::Nightly,
        mode: Mode::Debug,
        edition: Edition::E2018,
    };
    let mut warnings = false;

    if let Some(channel) = args.params.get("channel") {
        match channel.parse() {
            Ok(c) => flags.channel = c,
            Err(e) => errors += &format!("{}\n", e),
        }
    }

    if let Some(mode) = args.params.get("mode") {
        match mode.parse() {
            Ok(m) => flags.mode = m,
            Err(e) => errors += &format!("{}\n", e),
        }
    }

    if let Some(edition) = args.params.get("edition") {
        match edition.parse() {
            Ok(e) => flags.edition = e,
            Err(e) => errors += &format!("{}\n", e),
        }
    }

    if let Some(warn) = args.params.get("warn") {
        match warn.parse() {
            Ok(e) => warnings = e,
            Err(_) => errors += "invalid warn bool\n",
        }
    }

    (flags, warnings, errors)
}

fn send_play_result_reply(
    args: &Args,
    result: PlayResult,
    code: &str,
    flags: &CommandFlags,
    warn: bool,
    flag_parse_errors: &str,
) -> Result<(), Error> {
    let result = if warn {
        format!("{}\n{}", result.stderr, result.stdout)
    } else if result.success {
        result.stdout
    } else {
        result.stderr
    };

    if result.is_empty() {
        api::send_reply(&args, &format!("{}``` ```", flag_parse_errors))
    } else {
        crate::reply_potentially_long_text(
            &args,
            &format!("{}```\n{}", flag_parse_errors, result),
            "```",
            &format!(
                "Output too large. Playground link: {}",
                url_from_gist(&flags, &post_gist(&args, code)?),
            ),
        )
    }
}

// Generic function used for both `?eval` and `?play`
fn run_code_and_reply(args: &Args, code: &str) -> Result<(), Error> {
    let (flags, warn, flag_parse_errors) = parse_flags(args);

    let result: PlayResult = args
        .http
        .post("https://play.rust-lang.org/execute")
        .json(&PlaygroundRequest {
            code,
            channel: flags.channel,
            crate_type: if code.contains("fn main") {
                CrateType::Binary
            } else {
                CrateType::Library
            },
            edition: flags.edition,
            mode: flags.mode,
            tests: false,
        })
        .send()?
        .json()?;

    send_play_result_reply(args, result, code, &flags, warn, &flag_parse_errors)
}

pub fn play(args: Args) -> Result<(), Error> {
    match crate::extract_code(args.body) {
        Some(code) => run_code_and_reply(&args, code),
        None => crate::reply_missing_code_block_err(&args),
    }
}

pub fn eval(args: Args) -> Result<(), Error> {
    let code = match crate::extract_code(args.body) {
        Some(x) => x,
        None => return crate::reply_missing_code_block_err(&args),
    };

    if code.contains("fn main") {
        api::send_reply(&args, "code passed to ?eval should not contain `fn main`")?;
        return Ok(());
    }

    let mut full_code = String::from("fn main() {\n    println!(\"{:?}\", {\n");
    for line in code.lines() {
        full_code.push_str("        ");
        full_code.push_str(line);
        full_code.push_str("\n");
    }
    full_code.push_str("    });\n}");

    run_code_and_reply(&args, &full_code)
}

pub fn play_and_eval_help(args: Args, name: &str) -> Result<(), Error> {
    let message = format!(
        "Compile and run rust code. All code is executed on https://play.rust-lang.org.
```?{} mode={{}} channel={{}} edition={{}} warn={{}} ``\u{200B}`code``\u{200B}` ```
Optional arguments:
    \tmode: debug, release (default: debug)
    \tchannel: stable, beta, nightly (default: nightly)
    \tedition: 2015, 2018 (default: 2018)
    \twarn: boolean flag to enable compilation warnings
    ",
        name
    );

    api::send_reply(&args, &message)?;
    Ok(())
}

pub fn miri(args: Args) -> Result<(), Error> {
    let code = match crate::extract_code(args.body) {
        Some(x) => x,
        None => return crate::reply_missing_code_block_err(&args),
    };

    let (flags, warn, flag_parse_errors) = parse_flags(&args);

    let result: PlayResult = args
        .http
        .post("https://play.rust-lang.org/miri")
        .json(&MiriRequest {
            code,
            edition: flags.edition,
        })
        .send()?
        .json()?;

    send_play_result_reply(&args, result, code, &flags, warn, &flag_parse_errors)
}

pub fn miri_help(args: Args) -> Result<(), Error> {
    api::send_reply(
        &args,
        "Execute this program in the Miri interpreter to detect certain cases of undefined behavior
(like out-of-bounds memory access). All code is executed on https://play.rust-lang.org.
```?{} edition={{}} warn={{}} ``\u{200B}`code``\u{200B}` ```
Optional arguments:
    \tedition: 2015, 2018 (default: 2018)
    \twarn: boolean flag to enable compilation warnings",
    )?;
    Ok(())
}
