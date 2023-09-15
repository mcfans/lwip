use std::env;
use std::path::PathBuf;
use std::process::Command;

fn sdk_path(sdk: &str) -> String {
  let output = Command::new("xcrun")
    .arg("--sdk")
    .arg(sdk)
    .arg("--show-sdk-path")
    .output()
    .expect("failed to execute process");

  let stdout = String::from_utf8(output.stdout).unwrap();
  stdout.trim().to_string()
}

fn main() {
  println!("cargo:rustc-rerun-if-changed=wrapper.h");

  let mut builder = bindgen::Builder::default()
    // The input header we would like to generate
    // bindings for.
    .header("wrapper.h")
    .clang_arg("-I../include")
    .clang_arg("-I..")
    .clang_arg("-I/usr/include")
    .clang_arg("-I../../../../src/include")
    // Tell cargo to invalidate the built crate whenever any of the
    // included header files changed.
    .parse_callbacks(Box::new(bindgen::CargoCallbacks));

  if cfg!(target_os = "macos") {
    let sdk = "macosx";
    builder = builder.clang_arg(format!("-isysroot{}", sdk_path(sdk)));
  }
  if cfg!(target_os = "ios") {
    let sdk = "iphoneos";
    builder = builder.clang_arg(format!("-isysroot{}", sdk_path(sdk)));
  }

  let bindings = builder
    .generate()
    .expect("Unable to generate bindings");

  let out_path = PathBuf::from(env::var("OUT_DIR").unwrap());
  bindings
    .write_to_file(out_path.join("bindings.rs"))
    .expect("Couldn't write bindings!");
}
