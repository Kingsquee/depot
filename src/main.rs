#![feature(unicode)]
#![feature(path_ext)]

extern crate toml;
extern crate getopts;
extern crate rustc_serialize;
extern crate unicode;

use std::path::{PathBuf, Path};
use std::fs;
use std::fs::{File, PathExt};
use std::io::{Read, Write};
use std::collections::HashMap;
use std::process::{Command, Stdio};
use std::env;
use getopts::Options;

static DEPOT_TOML_NAME:           &'static str = "Depot.toml";
static DEPENDENCIES_TOML_NAME:    &'static str = "Dependencies.toml";
static DEFAULT_DEPOT_NAME:        &'static str = "depot";
static CARGO_NAME:                &'static str = "cargoproject";
static CARGO_PROJECT_TYPE:        &'static str = "lib";

static CARGO_DEFAULT_OPT_LEVEL:           &'static str = "3";
static CARGO_DEFAULT_DEBUG_FLAG:          &'static str = "false";
static CARGO_DEFAULT_DEBUG_ASSERTIONS:    &'static str = "false";

// Depot.toml
#[derive(RustcDecodable, RustcEncodable, Debug)]
struct DepotManifest {
  depot: DepotProject,
  settings: DepotProfile
}

#[derive(RustcDecodable, RustcEncodable, Debug)]
struct DepotProject {
  name: String,
  out_dir: Option<String>,
  dirs: Vec<String>
}

#[derive(RustcDecodable, RustcEncodable, Clone, Default, Debug)]
pub struct DepotProfile {
    opt_level: u32,
    // codegen_units: Option<u32>, // Is this something we want passed?
    debug: bool,
    debug_assertions: bool,
}

// Dependencies.toml
#[derive(RustcDecodable, RustcEncodable, Debug)]
struct DependencyManifest {
  dependencies: Option<HashMap<String, Dependency>>,
}

#[derive(RustcDecodable, RustcEncodable, Clone, Debug)]
enum Dependency {
  Simple(String),
  Detailed(DetailedDependency)
}

#[derive(RustcDecodable, RustcEncodable, Clone, Debug, Default)]
struct DetailedDependency {
  version: Option<String>,
  path: Option<String>,
  git: Option<String>,
  branch: Option<String>,
  tag: Option<String>,
  rev: Option<String>,
  features: Option<Vec<String>>,
  optional: Option<bool>,
  default_features: Option<bool>,
}

struct DependencyConflict {
  name: String,
  manifest_a: String,
  manifest_b: String,
  version_a: String,
  version_b: String,
}

fn print_usage(program: &str, opts: Options) {
    let brief = format!("Usage: {} [options]", program);
    print!("{}", opts.usage(&brief));
}

fn main() {

  let args: Vec<String> = env::args().collect();
  let program = args[0].clone();

  let mut opts = Options::new();
  opts.optopt("n", "name", "name of the Depot project", "NAME");
  opts.optmulti("d", "dir", "directory that contains a Dependency.toml", "DIR");
  opts.optopt("o", "out-dir", "directory that contains the output libraries", "OUTDIR");
  opts.optopt("", "opt-level", "optimize dependencies with possible levels 0-3", "OPTLEVEL");
  opts.optflag("", "debug", "passes -g and/or enables the debug configuration for the compiler");
  opts.optflag("", "debug-assertions", "enables debug assertions");
  opts.optflag("h", "help", "print this help menu");

  let matches = opts.parse(&args[1 ..]).unwrap();
  if matches.opt_present("h") {
    print_usage(&program, opts);
    return;
  }

  let current_dir = env::current_dir().unwrap();

  let mut depot_manifest: DepotManifest;
  let mut working_dir: PathBuf;

  // if the operator passed some dir args
  if matches.opt_present("d") {
    depot_manifest = DepotManifest {
      depot: DepotProject {
        dirs: matches.opt_strs("d"),
        name: matches.opt_str("n").unwrap_or(DEFAULT_DEPOT_NAME.to_string()),
        out_dir: Some(matches.opt_str("o").unwrap_or(current_dir.to_string_lossy().into_owned()))
      },
      settings: DepotProfile {
        opt_level: matches.opt_str("opt-level").unwrap_or(CARGO_DEFAULT_OPT_LEVEL.to_string()).parse().unwrap(),
        debug: matches.opt_str("debug").unwrap_or(CARGO_DEFAULT_DEBUG_FLAG.to_string()).parse().unwrap(),
        debug_assertions: matches.opt_str("debug-assertions").unwrap_or(CARGO_DEFAULT_DEBUG_ASSERTIONS.to_string()).parse().unwrap()
      }
    };
    working_dir = current_dir;

  // else if no args, we check the current directory for a depot config toml
  } else if matches.free.is_empty() {
    let depot_toml_path = current_dir.clone().join(DEPOT_TOML_NAME);
    depot_manifest = parse_depot_toml(&depot_toml_path);
    working_dir = current_dir;

  // else the operator should have passed a path to a depot config toml
  } else {
    let depot_toml_path = PathBuf::from(matches.free[0].clone());
    depot_manifest = parse_depot_toml(&depot_toml_path);
    working_dir = depot_toml_path.clone();
    working_dir.pop();
  };


  let mut dep_toml_dirs = Vec::new();
  for string in depot_manifest.depot.dirs.iter() {
    dep_toml_dirs.push(PathBuf::from(string));
  }

  let depot_project_name = depot_manifest.depot.name.clone();

  println!("Generating {}'s Cargo.toml.", depot_project_name);

  // Get the Dependency.tomls
  let mut dependency_tomls: Vec<PathBuf> = Vec::with_capacity(dep_toml_dirs.len());

  //TODO: Make this search subdirectories too
  for dir in dep_toml_dirs.iter() {
    let dependencies_toml = dir.clone().join(DEPENDENCIES_TOML_NAME);

    if dependencies_toml.exists() && dependencies_toml.is_file() {
      dependency_tomls.push(dependencies_toml);
      println!("Found {} in {:?}", DEPENDENCIES_TOML_NAME, dir);
    }
  }

  let mut toml_manifests = HashMap::new();

  // Extract the manifests from the tomls
  for toml_path in dependency_tomls.iter() {
    let mut toml_file = File::open(toml_path).unwrap();

    let mut toml_text = String::new();
    toml_file.read_to_string(&mut toml_text).unwrap();

    let root = parse(&toml_text, toml_path);
    let toml_manifest: DependencyManifest = toml::decode(toml::Value::Table(root)).unwrap();

    let dir_name = toml_path.parent().unwrap().file_name().unwrap().to_str().unwrap().to_string();
    toml_manifests.insert(dir_name, toml_manifest);
  }

  let mut conflicts = Vec::new();

  // Wow, this is ugly!
  // I should probably feel ashamed of myself!
  'a_manifest: for (manifest_name_a, manifest_a) in toml_manifests.iter() {
    if manifest_a.dependencies.is_none() { continue }
    'a_dep: for (dep_name_a, dep_a) in manifest_a.dependencies.as_ref().unwrap().iter() {
      'b_manifest: for (manifest_name_b, manifest_b) in toml_manifests.iter() {
        if manifest_b.dependencies.is_none() { continue }
        'b_dep: for (dep_name_b, dep_b) in manifest_b.dependencies.as_ref().unwrap().iter() {
          if manifest_name_a == manifest_name_b {
            continue 'a_manifest
          }
          if dep_name_a == dep_name_b {
            let version_a = get_version(dep_a);
            let version_b = get_version(dep_b);

            if version_a != version_b {
              conflicts.push(
                DependencyConflict {
                  name: dep_name_a.clone(),
                  manifest_a: manifest_name_a.clone(),
                  manifest_b: manifest_name_b.clone(),
                  version_a: version_a.clone(),
                  version_b: version_b.clone(),
                }
              );
            }
          }
        }
      }
    }
  }

  // Fix the hashmap ordering
  for conflict in conflicts.iter_mut() {
    if conflict.manifest_a > conflict.manifest_b {

      let tmp = conflict.manifest_a.clone();
      conflict.manifest_a = conflict.manifest_b.clone();
      conflict.manifest_b = tmp.clone();

      let tmp = conflict.version_a.clone();
      conflict.version_a = conflict.version_b.clone();
      conflict.version_b = tmp.clone();
    }
  }
  conflicts.sort_by(|a, b|
    a.manifest_a.cmp(&b.manifest_a)
  );

  for conflict in conflicts.iter() {
    // Display the conflict to the operator
    println!(
      "ERROR: Version mismatch: {} {}, while {} {}.",
      format!("\"{}\"", conflict.manifest_a),

      if conflict.version_a == "".to_string() {
        format!("uses the latest available version of {}", conflict.name)
      } else {
        format!("requires {} of {}", conflict.version_a, conflict.name)
      },

      format!("\"{}\"", conflict.manifest_b),

      if conflict.version_b == "".to_string() {
        format!("uses the latest available version of {}", conflict.name)
      } else {
        format!("requires {} of {}", conflict.version_b, conflict.name)
      }
    );
  }

  if conflicts.len() != 0 {
    println!("You may be able to resolve these conflicts by modifying their respective \"{}\"\'s.", DEPENDENCIES_TOML_NAME);
    panic!();
  }

  let mut final_manifest = DependencyManifest { dependencies: Some(HashMap::new()) };
  for (_, manifest) in toml_manifests.iter() {
    if manifest.dependencies.is_none() {
      continue
    }
    for (dep_name, dep) in manifest.dependencies.as_ref().unwrap().iter() {
      final_manifest.dependencies.as_mut().unwrap().insert(dep_name.clone(), dep.clone());
    }
  }

  let cargo_toml_text =
format!("#ATTENTION: This file is automatically generated. Don't modify it unless your life is terrible, or you wish it to be so.
[package]
name = \"{project_name}\"
version = \"0.0.1\"
authors = [ \"automatically generated\" ]

[lib]
name = \"{project_name}\"
crate_type = [\"{crate_type}\"]
plugin = true

[profile.dev]
opt-level = {opt_level}
debug = {debug}
debug_assertions = {debug_assertions}

{dependencies}
",
project_name = CARGO_NAME,
opt_level = depot_manifest.settings.opt_level,
debug = depot_manifest.settings.debug,
debug_assertions = depot_manifest.settings.debug_assertions,
dependencies = toml::encode_str(&final_manifest),
crate_type = CARGO_PROJECT_TYPE);

  let depot_dir = match &depot_manifest.depot.out_dir {
    &Some(ref string) => PathBuf::from(string),
    &None => PathBuf::from(working_dir),
  }.join(&depot_project_name);

  let cargo_project_name = depot_manifest.depot.name.clone() + "-" + CARGO_NAME;
  let hidden_cargo_dir_name = ".".to_string() + &cargo_project_name;
  let cargo_dir = depot_dir.join(&hidden_cargo_dir_name);

  let cargo_toml_path = cargo_dir.join("Cargo.toml");

  let cargo_deps = cargo_dir.join("target").join("debug").join("deps");
  let cargo_native = cargo_dir.join("target").join("debug").join("native");

  let depot_deps = depot_dir.join("deps");
  let depot_native = depot_dir.join("native");

  // if it exists already
  if cargo_toml_path.exists() && cargo_toml_path.is_file() {

    println!("Creating new Cargo.toml: {:?}", cargo_toml_path);
    let mut cargo_toml_file = File::create(&cargo_toml_path).unwrap();
    cargo_toml_file.write_all(cargo_toml_text.as_bytes()).unwrap();
    cargo_toml_file.flush().unwrap();

    //move dep and native folders from the depot dir to the cargo project dir, for updating
    fs::rename(&depot_deps, &cargo_deps).unwrap();
    fs::rename(&depot_native, &cargo_native).unwrap();

    Command::new("cargo")
      .arg("build")
      .current_dir(&cargo_dir)
      .stdout(Stdio::inherit())
      .stderr(Stdio::inherit())
      .output().unwrap_or_else(|e| {
        panic!("failed to run cargo: {}", e)
      });

    //move dep and native folders BACK to the depot dir, for easy referencing
    fs::rename(&cargo_deps, &depot_deps).unwrap();
    fs::rename(&cargo_native, &depot_native).unwrap();

    // Done!
  } else {
    fs::create_dir(&depot_dir).unwrap();
    Command::new("cargo")
      .arg("new")
      .arg(&cargo_project_name)
      .current_dir(&depot_dir)
      .stdout(Stdio::inherit())
      .stderr(Stdio::inherit())
      .output().unwrap_or_else(|e| {
      panic!("failed to run cargo: {}", e)
    });

    fs::rename(depot_dir.join(&cargo_project_name), depot_dir.join(&hidden_cargo_dir_name)).unwrap();

    let mut cargo_toml_file = File::create(&cargo_toml_path).unwrap();
    cargo_toml_file.write_all(cargo_toml_text.as_bytes()).unwrap();
    cargo_toml_file.flush().unwrap();

    Command::new("cargo")
      .arg("build")
      .stdout(Stdio::inherit())
      .stderr(Stdio::inherit())
      .current_dir(&cargo_dir)
      .output().unwrap_or_else(|e| {
        panic!("failed to run cargo: {}", e)
      });

    //move dep and native folders from the cargo project dir to the depot dir, for easier referencing
    fs::rename(&cargo_deps, &depot_deps).unwrap();
    fs::rename(&cargo_native, &depot_native).unwrap();

  // Done!
  }
  println!("Build complete.");

}

fn get_version(d: &Dependency) -> String {
  match d {
    &Dependency::Simple(ref version) => {
      version.clone()
    }
    &Dependency::Detailed(ref details) => {
      match details.version {
        Some(ref version) => version.clone(),
        None => "".to_string()
      }
    }
  }
}

fn parse_depot_toml(depot_toml_path: &Path) -> DepotManifest{

  let mut toml_file = File::open(depot_toml_path).unwrap();

  let mut toml_text = String::new();
  toml_file.read_to_string(&mut toml_text).unwrap();

  let root = parse(&toml_text, depot_toml_path);
  let toml_manifest: DepotManifest = toml::decode(toml::Value::Table(root)).unwrap();
  toml_manifest
}

pub fn parse(toml: &str, file: &Path) -> toml::Table {
  let mut parser = toml::Parser::new(&toml);
  match parser.parse() {
    Some(toml) => return toml,
    None => {}
  }
  let mut error_str = format!("could not parse input TOML\n");
  for error in parser.errors.iter() {
    let (loline, locol) = parser.to_linecol(error.lo);
    let (hiline, hicol) = parser.to_linecol(error.hi);
    error_str.push_str(
      &format!("{}:{}:{}{} {}\n",
        file.display(),
        loline + 1,
        locol + 1,
        if loline != hiline || locol != hicol {
          format!("-{}:{}", hiline + 1, hicol + 1)
        } else {
          "".to_string()
        },
        error.desc
      )
    );
  }
  panic!("{}", error_str);
}