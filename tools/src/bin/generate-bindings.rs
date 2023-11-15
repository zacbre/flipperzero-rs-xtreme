//! Generate bindings.rs for Flipper Zero SDK.
//!
//! Usage: `generate-bindings flipperzero-firmware/build/f7-firmware-D/sdk/`

use std::{env, fs};

use bindgen::callbacks::ParseCallbacks;
use camino::{Utf8Path, Utf8PathBuf};
use clap::{self, crate_authors, crate_description, crate_version, value_parser};
use serde::Deserialize;

const TARGET: &str = "thumbv7em-none-eabihf";
const OUTFILE: &str = "bindings.rs";
const SDK_OPTS: &str = "sdk.opts";
#[cfg(all(windows, target_arch = "x86"))]
const TOOLCHAIN: &str = "../../../toolchain/i686-windows/arm-none-eabi/include";
#[cfg(all(windows, target_arch = "x86_64"))]
const TOOLCHAIN: &str = "../../../toolchain/x86_64-windows/arm-none-eabi/include";
#[cfg(all(unix, target_arch = "x86"))]
const TOOLCHAIN: &str = "../../../toolchain/i686-linux/arm-none-eabi/include";
#[cfg(all(unix, target_arch = "x86_64"))]
const TOOLCHAIN: &str = "../../../toolchain/x86_64-linux/arm-none-eabi/include";
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
const TOOLCHAIN: &str = "~/.ufbt/toolchain/x86_64-darwin/arm-none-eabi/include";
const VISIBILITY_PUBLIC: &str = "+";

#[derive(Debug)]
struct ApiSymbols {
    pub api_version: u32,
    pub headers: Vec<String>,
    pub functions: Vec<String>,
    pub variables: Vec<String>,
}

/// Load symbols from `api_symbols.csv`.
fn load_symbols<T: AsRef<Utf8Path>>(path: T) -> ApiSymbols {
    let path = path.as_ref();

    let mut reader = csv::Reader::from_path(path).expect("failed to load symbol file");

    let mut api_version: u32 = 0;
    let mut headers = Vec::new();
    let mut functions = Vec::new();
    let mut variables = Vec::new();

    for record in reader.records() {
        let record = record.expect("failed to parse symbol record");
        let name = &record[0];
        let visibility = &record[1];
        let value = &record[2];

        if visibility != VISIBILITY_PUBLIC {
            continue;
        }

        match name {
            "Version" => {
                let v = value
                    .split_once('.')
                    .expect("failed to parse symbol version");
                let major: u16 = v.0.parse().unwrap();
                let minor: u16 = v.1.parse().unwrap();

                api_version = ((major as u32) << 16) | (minor as u32);
            }
            "Header" => headers.push(value.to_string()),
            "Function" => functions.push(value.to_string()),
            "Variable" => variables.push(value.to_string()),
            _ => (),
        }
    }

    ApiSymbols {
        api_version,
        headers,
        functions,
        variables,
    }
}

#[allow(dead_code)]
#[derive(Debug, Deserialize)]
struct SdkOpts {
    sdk_symbols: String,
    cc_args: String,
}

/// Load `sdk.opts` file of compiler flags.
fn load_sdk_opts<T: AsRef<Utf8Path>>(path: T) -> SdkOpts {
    let file = fs::File::open(path.as_ref()).expect("failed to open sdk.opts");

    let sdk_opts: SdkOpts = serde_json::from_reader(file).expect("failed to parse sdk.opts JSON");

    sdk_opts
}

/// Generate bindings header.
fn generate_bindings_header(api_symbols: &ApiSymbols) -> String {
    let mut lines = Vec::new();

    lines.push(format!(
        "#define API_VERSION 0x{:08X}",
        api_symbols.api_version
    ));
    lines.push("#include \"furi/furi.h\"".to_string());

    for header in &api_symbols.headers {
        lines.push(format!("#include \"{header}\""))
    }

    lines.join("\n")
}

/// Parse command-line arguments.
fn parse_args() -> clap::ArgMatches {
    clap::Command::new("generate-bindings")
        .version(crate_version!())
        .author(crate_authors!())
        .about(crate_description!())
        .arg(clap::Arg::new("sdk").value_parser(value_parser!(Utf8PathBuf)))
        .get_matches()
}

#[derive(Debug)]
struct Cb;

impl ParseCallbacks for Cb {
    fn process_comment(&self, comment: &str) -> Option<String> {
        Some(doxygen_rs::transform(comment))
    }
}

fn main() {
    let matches = parse_args();

    let sdk = matches
        .get_one::<Utf8PathBuf>("sdk")
        .expect("failed to find SDK directory");

    if !sdk.is_dir() {
        panic!("No such directory: {}", sdk);
    }

    // We must provide absolute paths to Clang. Unfortunately on Windows
    // `Path::canonicalize` returns a `\\?\C:\...` style path that is not
    // compatible with Clang.
    let cwd = Utf8PathBuf::try_from(env::current_dir().unwrap()).unwrap();
    let sdk = cwd.join(sdk);

    let toolchain = sdk.join(TOOLCHAIN);
    if !toolchain.is_dir() {
        panic!(
            concat!(
                "Failed to find toolchain at {:?}.\n",
                "You may need to download it first."
            ),
            TOOLCHAIN
        )
    }

    let replace_sdk_root_dir = |s: &str| {
        // Need to use '/' on Windows, or else include paths don't work
        s.replace("SDK_ROOT_DIR", sdk.as_str()).replace('\\', "/")
    };

    // Load SDK compiler flags
    let sdk_opts = load_sdk_opts(sdk.join(SDK_OPTS));

    // Load SDK symbols
    let symbols = load_symbols(sdk.join(replace_sdk_root_dir(&sdk_opts.sdk_symbols)));
    let bindings_header = generate_bindings_header(&symbols);

    // Some of the values are shell-quoted
    let cc_flags = shlex::split(&sdk_opts.cc_args).expect("failed to split sdk.opts cc_args");
    let cc_flags: Vec<String> = cc_flags
        .into_iter()
        .map(|arg| {
            match arg.as_str() {
                // Force word relocations by disallowing MOVW / MOVT
                "-mword-relocations" => String::from("-mno-movt"),
                a => replace_sdk_root_dir(a),
            }
        })
        .collect();

    // Generate bindings
    eprintln!("Generating bindings for SDK {:08X}", symbols.api_version);
    let mut bindings = bindgen::builder()
        .clang_args(["-target", TARGET])
        .clang_args(["-working-directory", sdk.as_str()])
        .clang_args(["--system-header-prefix=f7_sdk/"])
        .clang_args(["-isystem", toolchain.as_str()])
        .clang_args(cc_flags)
        .clang_arg("-Wno-error")
        .clang_arg("-fshort-enums")
        .clang_arg("-fvisibility=default")
        .use_core()
        .parse_callbacks(Box::new(Cb))
        .ctypes_prefix("core::ffi")
        .allowlist_var("API_VERSION")
        .header_contents("header.h", &bindings_header);

    for function in &symbols.functions {
        bindings = bindings.allowlist_function(function);
    }

    for variable in &symbols.variables {
        bindings = bindings.allowlist_var(variable);
    }

    let bindings = match bindings.generate() {
        Ok(b) => b,
        Err(e) => {
            // Separate error output from the preceding clang diag output for legibility
            println!("\n{e}");
            panic!("failed to generate bindings")
        }
    };

    // `-working-directory` also affects `Bindings::write_to_file`
    let outfile = cwd.join(OUTFILE);

    eprintln!("Writing to {OUTFILE:?}");
    bindings
        .write_to_file(outfile)
        .expect("failed to write bindings");
}
