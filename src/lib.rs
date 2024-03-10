#![deny(clippy::all)]
use es_module_lexer::lex;

use napi::bindgen_prelude::JsFunction;
use napi::{Env, Result};
use napi::{JsBoolean, JsObject, JsString, JsUndefined, JsUnknown};
use oxc_resolver::{ResolveOptions, Resolver};
use pathdiff::diff_paths;
use regex::Regex;
use std::collections::HashMap;
use std::path::PathBuf;
use url::Url;

// @TODO https://github.com/devongovett/glob-match
// for excludePattern

use napi_derive::napi;

pub fn is_bare_module_specifier(specifier: &str) -> bool {
  let specifier = specifier.replace('\'', "");
  if let Some(first_char) = specifier.chars().next() {
    let re = Regex::new(r"[@a-zA-Z]").unwrap();
    return re.is_match(&first_char.to_string());
  }
  false
}

pub fn is_scoped_package(specifier: &str) -> bool {
  specifier.starts_with('@')
}

#[napi(object)]
#[derive(Debug)]
pub struct ModuleGraph {
  pub graph: HashMap<String, Vec<String>>,
  pub base_path: String,
  pub entry_points: Vec<String>,
  pub modules: HashMap<String, Module>,
}

#[napi(object)]
#[derive(Default, Debug)]
pub struct PackageJson {
  pub name: Option<String>,
  pub version: Option<String>,
  pub path: String,
  pub href: String,
}

#[napi(object)]
#[derive(Default, Debug)]
pub struct Module {
  pub href: String,
  pub pathname: String,
  pub path: String,
  pub imported_by: Vec<String>,
  pub package_json: PackageJson,
  // pub facade: bool,
  // pub has_module_syntax: bool,
  pub source: String,
}

#[napi(object)]
pub struct Plugin {
  pub name: Option<String>,
  pub start: Option<JsFunction>,
  pub analyze: Option<JsFunction>,
  pub resolve: Option<JsFunction>,
  pub handle_import: Option<JsFunction>,
}

#[napi]
#[allow(clippy::too_many_arguments)]
pub fn create_module_graph(
  env: Env,
  entry_points: Vec<String>,
  base_path: String,
  condition_names: Vec<String>,
  builtin_modules: Vec<String>,
  ignore_external: bool,
  plugins: Vec<Plugin>,
  dev: bool,
) -> Result<ModuleGraph> {
  let options = ResolveOptions {
    condition_names,
    ..ResolveOptions::default()
  };

  let mut module_graph = ModuleGraph {
    graph: HashMap::new(),
    base_path,
    entry_points,
    modules: HashMap::new(),
  };

  let resolver = Resolver::new(options);

  let mut modules = Vec::new();

  for file_path in &module_graph.entry_points {
    let resolved_url = resolver
      .resolve(&module_graph.base_path, file_path)
      .unwrap();

    let module_path = diff_paths(resolved_url.full_path(), &module_graph.base_path).unwrap();

    let m = PathBuf::from(&module_graph.base_path).join(&module_path);

    let p = resolved_url.package_json().unwrap();
    let package_json = PackageJson {
      name: p
        .raw_json()
        .get("name")
        .map(|v| v.as_str().unwrap().to_string()),
      version: p
        .raw_json()
        .get("version")
        .map(|v| v.as_str().unwrap().to_string()),
      path: p.path.to_str().unwrap().to_string(),
      href: Url::from_file_path(&p.path).unwrap().to_string(),
    };

    module_graph
      .modules
      .entry(module_path.to_str().unwrap().to_string())
      .or_insert(Module {
        href: Url::from_file_path(&m).unwrap().to_string(),
        pathname: m.to_str().unwrap().to_string(),
        path: module_path.to_str().unwrap().to_string(),
        imported_by: vec![],
        // @TODO we dont pass the source yet
        source: "".to_string(),
        package_json,
      });

    modules.push(module_path);
  }

  // clone because we mutate the `modules` array above to iterate
  module_graph.entry_points = modules
    .clone()
    .into_iter()
    .map(|f| f.to_str().unwrap().to_string())
    .collect::<Vec<String>>();

  for plugin in &plugins {
    plugin.name.as_ref().expect("Plugin must have a name");

    let entry_points_js: Vec<JsString> = module_graph
      .entry_points
      .iter()
      .map(|s| env.create_string(s).unwrap())
      .collect();
    let base_path = env.create_string(&module_graph.base_path)?;

    if let Some(start) = &plugin.start {
      start.call2::<Vec<JsString>, JsString, JsUndefined>(entry_points_js, base_path)?;
    }
  }
  // checks for dynamic expressions in importee, e.g.: "`./translations/${locale}.js`"
  // can probably remove once we use oxc_module_lexer
  let re = Regex::new(r"\$\{[^}]+\}").unwrap();

  while let Some(dep) = modules.pop() {
    let source =
      std::fs::read_to_string(PathBuf::from(&module_graph.base_path).join(&dep)).unwrap();
    let module = lex(&source).expect("Failed to lex");

    // Add `dep` to the graph
    module_graph
      .graph
      .entry(dep.to_str().unwrap().to_string())
      .or_default();

    // @TODO use oxc_module_lexer
    // https://github.com/oxc-project/oxc/blob/49a4e6ecf274222b27c13bdce0e0ef37413dd060/crates/oxc_module_lexer/examples/module_lexer.rs
    'importloop: for import in module.imports() {
      let mut importee = import.specifier().to_string();
      if dev {
        println!("0 base_path: {:#?}", &module_graph.base_path);
        println!("1 importee: {:#?}", importee);
        println!("2 dep: {:#?}", dep);
      }
      if importee.is_empty()
        || importee == "import.meta"
        || builtin_modules.contains(&importee.replace("node:", ""))
      {
        continue;
      }
      if is_bare_module_specifier(&importee) && ignore_external {
        continue;
      }
      // checks for dynamic expressions in importee, e.g.: "`./translations/${locale}.js`"
      // can probably remove once we use oxc_module_lexer
      if re.is_match(&importee) {
        continue;
      }

      for plugin in &plugins {
        if let Some(handle_import) = &plugin.handle_import {
          let result = handle_import.call2::<JsString, JsString, JsUnknown>(
            env.create_string(dep.to_str().unwrap())?,
            env.create_string(&importee)?,
          )?;

          match &result.get_type()? {
            napi::ValueType::String => {
              let js_string: JsString = result.coerce_to_string()?;
              importee = js_string.into_utf8()?.as_str()?.to_string();
            }
            napi::ValueType::Boolean => {
              let js_bool: JsBoolean = result.coerce_to_bool()?;
              if !(js_bool.get_value()?) {
                continue 'importloop;
              }
            }
            _ => {
              // println!("Expected a string or a boolean");
            }
          }
        }
      }

      let importer = PathBuf::from(&module_graph.base_path).join(&dep);
      if dev {
        println!("3 importer: {:#?}", importer);
      }

      let resolved_url = resolver
        .resolve(importer.parent().unwrap().to_str().unwrap(), &importee)
        .unwrap();

      let path_to_dependency = diff_paths(resolved_url.path(), &module_graph.base_path).unwrap();
      if dev {
        println!("4 resolved_url: {:#?}", resolved_url.path());
        println!("5 path_to_dependency: {:#?}", path_to_dependency);
        println!("-------------------");
      }

      let dep_str = dep.to_str().unwrap().to_string();
      let path_to_dependency_str = path_to_dependency.to_str().unwrap().to_string();
      let resolved_path_str = resolved_url.path().to_str().unwrap().to_string();

      let p = resolved_url.package_json().unwrap();
      let package_json = PackageJson {
        name: p
          .raw_json()
          .get("name")
          .map(|v| v.as_str().unwrap().to_string()),
        version: p
          .raw_json()
          .get("version")
          .map(|v| v.as_str().unwrap().to_string()),
        path: p.path.to_str().unwrap().to_string(),
        href: Url::from_file_path(&p.path).unwrap().to_string(),
      };

      let module = Module {
        href: Url::from_file_path(&resolved_path_str).unwrap().to_string(),
        pathname: resolved_path_str.clone(),
        path: path_to_dependency_str.clone(),
        imported_by: vec![dep_str.clone()],
        source: source.clone(),
        package_json,
      };

      // for plugin in &plugins {
      //   if let Some(analyze) = &plugin.analyze {
      //     analyze.call::<JsObject>(None, &[module])?;
      //   }
      // }

      if !module_graph.graph.contains_key(&path_to_dependency_str) {
        modules.push(path_to_dependency.clone());
      }

      module_graph
        .modules
        .entry(path_to_dependency_str.clone())
        .or_insert(module);

      module_graph
        .graph
        .get_mut(&dep_str)
        .unwrap()
        .push(path_to_dependency_str);
    }
  }

  Ok(module_graph)
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn test_is_scoped_package() {
    assert!(is_scoped_package("@foo/bar"));
    assert!(!is_scoped_package("foo"));
    assert!(!is_scoped_package("/foo"));
    assert!(!is_scoped_package("./foo"));
  }

  #[test]
  fn test_is_bare_module_specifier() {
    assert!(is_bare_module_specifier("@foo"));
    assert!(is_bare_module_specifier("bar"));
    assert!(!is_bare_module_specifier("/baz"));
    assert!(!is_bare_module_specifier("./qux"));
  }
}

#[napi(object)]
pub struct Foo {
  pub bar: String,
  pub baz: Vec<String>,
}

#[napi]
pub fn run_js_callback<T: Fn(Foo) -> Result<()>>(callback: T) {
  callback(Foo {
    bar: "bar".to_string(),
    baz: vec!["baz".to_string()],
  })
  .unwrap();
}
