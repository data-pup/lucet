use failure::{Error, Fail};
use std::env;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use tempfile::TempDir;

const WASI_TARGET: &str = "wasm32-unknown-wasi";

#[derive(Debug, Fail)]
pub enum CompileError {
    #[fail(display = "File not found: {}", _0)]
    FileNotFound(String),
    #[fail(display = "Clang reported error: {}", _0)]
    Execution { stdout: String, stderr: String },
    #[fail(display = "Lucetc error: {}", _0)]
    Lucetc {
        #[cause]
        e: Error,
    },
    #[fail(display = "IO error: {}", _0)]
    IO {
        #[cause]
        e: std::io::Error,
    },
}

impl From<std::io::Error> for CompileError {
    fn from(e: std::io::Error) -> CompileError {
        CompileError::IO { e }
    }
}

impl CompileError {
    pub fn check(output: Output, print: bool) -> Result<(), Self> {
        if print {
            std::io::stdout().write_all(&output.stdout).unwrap();
            std::io::stderr().write_all(&output.stderr).unwrap();
        }
        if output.status.success() {
            Ok(())
        } else {
            Err(CompileError::Execution {
                stdout: String::from_utf8_lossy(&output.stdout).to_string(),
                stderr: String::from_utf8_lossy(&output.stderr).to_string(),
            })
        }
    }
}

fn wasi_sdk() -> PathBuf {
    Path::new(&env::var("WASI_SDK").unwrap_or("/opt/wasi-sdk".to_owned())).to_path_buf()
}

fn wasi_sysroot() -> PathBuf {
    match env::var("WASI_SYSROOT") {
        Ok(wasi_sysroot) => Path::new(&wasi_sysroot).to_path_buf(),
        Err(_) => {
            let mut path = wasi_sdk();
            path.push("share");
            path.push("sysroot");
            path
        }
    }
}

fn wasm_clang() -> PathBuf {
    match env::var("CLANG") {
        Ok(clang) => Path::new(&clang).to_path_buf(),
        Err(_) => {
            let mut path = wasi_sdk();
            path.push("bin");
            path.push("clang");
            path
        }
    }
}

pub struct Compile {
    input: PathBuf,
    cflags: Vec<String>,
    print_output: bool,
}

pub trait CompileOpts {
    fn cflag<S: AsRef<str>>(&mut self, cflag: S);
    fn with_cflag<S: AsRef<str>>(self, cflag: S) -> Self;

    fn include<S: AsRef<str>>(&mut self, include: S);
    fn with_include<S: AsRef<str>>(self, include: S) -> Self;
}

impl CompileOpts for Compile {
    fn cflag<S: AsRef<str>>(&mut self, cflag: S) {
        self.cflags.push(cflag.as_ref().to_owned());
    }

    fn with_cflag<S: AsRef<str>>(mut self, cflag: S) -> Self {
        self.cflag(cflag);
        self
    }

    fn include<S: AsRef<str>>(&mut self, include: S) {
        self.cflags.push(format!("-I{}", include.as_ref()));
    }

    fn with_include<S: AsRef<str>>(mut self, include: S) -> Self {
        self.include(include);
        self
    }
}

impl Compile {
    pub fn new<P: AsRef<Path>>(input: P) -> Self {
        Compile {
            input: PathBuf::from(input.as_ref()),
            cflags: Vec::new(),
            print_output: false,
        }
    }

    pub fn print_output(&mut self, print: bool) {
        self.print_output = print;
    }

    pub fn with_print_output(mut self, print: bool) -> Self {
        self.print_output(print);
        self
    }

    pub fn compile<P: AsRef<Path>>(&self, output: P) -> Result<(), CompileError> {
        let clang = wasm_clang();
        if !clang.exists() {
            Err(CompileError::FileNotFound(
                clang.to_string_lossy().into_owned(),
            ))?;
        }
        if !self.input.exists() {
            Err(CompileError::FileNotFound(
                self.input.to_string_lossy().into_owned(),
            ))?;
        }
        let mut cmd = Command::new(clang);
        cmd.arg(format!("--target={}", WASI_TARGET));
        cmd.arg(format!("--sysroot={}", wasi_sysroot().display()));
        cmd.arg("-c");
        cmd.arg(self.input.clone());
        cmd.arg("-o");
        cmd.arg(output.as_ref());
        for cflag in self.cflags.iter() {
            cmd.arg(cflag);
        }
        let run = cmd.output().expect("clang executable exists");
        CompileError::check(run, self.print_output)
    }
}

pub struct Link {
    input: Vec<PathBuf>,
    cflags: Vec<String>,
    ldflags: Vec<String>,
    print_output: bool,
}

impl Link {
    pub fn new<P: AsRef<Path>>(input: &[P]) -> Self {
        Link {
            input: input.iter().map(|p| PathBuf::from(p.as_ref())).collect(),
            cflags: vec![],
            ldflags: vec![],
            print_output: false,
        }
        .with_link_opt(LinkOpt::DefaultOpts)
    }

    pub fn print_output(&mut self, print: bool) {
        self.print_output = print;
    }

    pub fn with_print_output(mut self, print: bool) -> Self {
        self.print_output(print);
        self
    }

    pub fn link<P: AsRef<Path>>(&self, output: P) -> Result<(), CompileError> {
        let clang = wasm_clang();
        if !clang.exists() {
            Err(CompileError::FileNotFound(
                clang.to_string_lossy().into_owned(),
            ))?;
        }
        let mut cmd = Command::new(clang);
        for input in self.input.iter() {
            if !input.exists() {
                Err(CompileError::FileNotFound(
                    input.to_string_lossy().into_owned(),
                ))?;
            }
            cmd.arg(input.clone());
        }
        cmd.arg("-o");
        cmd.arg(output.as_ref());
        for cflag in self.cflags.iter() {
            cmd.arg(cflag);
        }
        for ldflag in self.ldflags.iter() {
            cmd.arg(format!("-Wl,{}", ldflag));
        }
        let run = cmd.output().expect("clang executable exists");
        CompileError::check(run, self.print_output)
    }
}

pub trait AsLink {
    fn as_link(&mut self) -> &mut Link;
}

impl AsLink for Link {
    fn as_link(&mut self) -> &mut Link {
        self
    }
}

#[derive(Clone, Copy, Debug)]
pub enum LinkOpt<'t> {
    /// Allow references to an undefined function that will be resolved later by the dynamic linker
    AllowUndefined(&'t str),

    /// Allow references to any undefined function. They will be resolved later by the dynamic linker
    AllowUndefinedAll,

    /// Default options, possibly enabling workarounds for temporary bugs
    DefaultOpts,

    /// Export a symbol
    Export(&'t str),

    /// Preserve all the symbols during LTO, even if they are not used
    ExportAll,

    /// Do not assume that the library has a predefined entry point
    NoDefaultEntryPoint,

    /// Create a shared library
    Shared,

    /// Do not put debug information (STABS or DWARF) in the output file
    StripDebug,

    /// Remove functions and data that are unreachable by the entry point or exported symbols
    StripUnused,
}

impl<'t> LinkOpt<'t> {
    #[cfg(target_os = "macos")]
    fn as_ldflags(&self) -> Vec<String> {
        match self {
            LinkOpt::AllowUndefined(_symbol) => vec![],
            LinkOpt::AllowUndefinedAll => vec!["-undefined,dynamic_lookup".to_string()],
            LinkOpt::DefaultOpts => vec![],
            LinkOpt::Export(symbol) => vec![format!("-exported_symbol,{}", symbol).to_string()],
            LinkOpt::ExportAll => vec!["-export_dynamic".to_string()],
            LinkOpt::NoDefaultEntryPoint => vec![],
            LinkOpt::Shared => vec!["-dylib".to_string()],
            LinkOpt::StripDebug => vec!["-S".to_string()],
            LinkOpt::StripUnused => vec!["-dead_strip".to_string()],
        }
    }

    #[cfg(not(target_os = "macos"))]
    fn as_ldflags(&self) -> Vec<String> {
        match self {
            LinkOpt::AllowUndefined(symbol) => vec![format!("-U,_{}", symbol).to_string()],
            LinkOpt::AllowUndefinedAll => vec!["--allow-undefined".to_string()],
            LinkOpt::DefaultOpts => vec!["--no-threads".to_string()],
            LinkOpt::Export(symbol) => vec![format!("--export={}", symbol).to_string()],
            LinkOpt::ExportAll => vec!["--export-all".to_string()],
            LinkOpt::NoDefaultEntryPoint => vec!["--no-entry".to_string()],
            LinkOpt::Shared => vec!["--shared".to_string()],
            LinkOpt::StripDebug => vec!["-S".to_string()],
            LinkOpt::StripUnused => vec!["--strip-discarded".to_string()],
        }
    }
}

pub trait LinkOpts {
    fn link_opt(&mut self, link_opt: LinkOpt);
    fn with_link_opt(self, link_opt: LinkOpt) -> Self;

    fn export<S: AsRef<str>>(&mut self, export: S);
    fn with_export<S: AsRef<str>>(self, export: S) -> Self;
}

impl<T: AsLink> LinkOpts for T {
    fn link_opt(&mut self, link_opt: LinkOpt) {
        self.as_link().ldflags.extend(link_opt.as_ldflags());
    }

    fn with_link_opt(mut self, link_opt: LinkOpt) -> Self {
        self.link_opt(link_opt);
        self
    }

    fn export<S: AsRef<str>>(&mut self, export: S) {
        self.link_opt(LinkOpt::Export(export.as_ref()));
    }

    fn with_export<S: AsRef<str>>(mut self, export: S) -> Self {
        self.export(export);
        self
    }
}

impl<T: AsLink> CompileOpts for T {
    fn cflag<S: AsRef<str>>(&mut self, cflag: S) {
        self.as_link().cflags.push(cflag.as_ref().to_owned());
    }

    fn with_cflag<S: AsRef<str>>(mut self, cflag: S) -> Self {
        self.cflag(cflag);
        self
    }

    fn include<S: AsRef<str>>(&mut self, include: S) {
        self.as_link()
            .cflags
            .push(format!("-I{}", include.as_ref()));
    }

    fn with_include<S: AsRef<str>>(mut self, include: S) -> Self {
        self.include(include);
        self
    }
}

pub struct Lucetc {
    link: Link,
    lucetc: lucetc::Lucetc,
    tmpdir: TempDir,
    wasm_file: PathBuf,
}

impl Lucetc {
    pub fn new<P: AsRef<Path>>(input: &[P]) -> Self {
        let link = Link::new(input);
        let tmpdir = TempDir::new().expect("temporary directory creation failed");
        let wasm_file = tmpdir.path().join("out.wasm");
        let lucetc = lucetc::Lucetc::new(&wasm_file);
        Lucetc {
            link,
            lucetc,
            tmpdir,
            wasm_file,
        }
    }

    pub fn print_output(mut self, print: bool) -> Self {
        self.link.print_output = print;
        self
    }

    pub fn build<P: AsRef<Path>>(self, output: P) -> Result<(), CompileError> {
        self.link.link(&self.wasm_file)?;
        self.lucetc
            .shared_object_file(output.as_ref())
            .map_err(|e| CompileError::Lucetc { e })?;
        Ok(self.tmpdir.close()?)
    }
}

impl AsLink for Lucetc {
    fn as_link(&mut self) -> &mut Link {
        &mut self.link
    }
}

impl lucetc::AsLucetc for Lucetc {
    fn as_lucetc(&mut self) -> &mut lucetc::Lucetc {
        &mut self.lucetc
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;
    #[test]
    fn wasi_sdk_installed() {
        let clang = wasm_clang();
        assert!(clang.exists(), "clang executable exists");
    }

    fn test_file(name: &str) -> PathBuf {
        let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        p.push("test");
        p.push(name);
        assert!(p.exists(), "test file does not exist");
        p
    }

    #[test]
    fn compile_a() {
        let tmp = TempDir::new().expect("create temporary directory");

        let compiler = Compile::new(test_file("a.c"));

        let objfile = tmp.path().join("a.o");

        compiler.compile(objfile.clone()).expect("compile a.c");

        assert!(objfile.exists(), "object file created");

        let mut linker = Link::new(&[objfile]);
        linker.cflag("-nostartfiles");
        linker.link_opt(LinkOpt::NoDefaultEntryPoint);

        let wasmfile = tmp.path().join("a.wasm");

        linker.link(wasmfile.clone()).expect("link a.wasm");

        assert!(wasmfile.exists(), "wasm file created");
    }

    #[test]
    fn compile_b() {
        let tmp = TempDir::new().expect("create temporary directory");

        let compiler = Compile::new(test_file("b.c"));

        let objfile = tmp.path().join("b.o");

        compiler.compile(objfile.clone()).expect("compile b.c");

        assert!(objfile.exists(), "object file created");

        let mut linker = Link::new(&[objfile]);
        linker.cflag("-nostartfiles");
        linker.link_opt(LinkOpt::NoDefaultEntryPoint);
        linker.link_opt(LinkOpt::AllowUndefinedAll);

        let wasmfile = tmp.path().join("b.wasm");

        linker.link(wasmfile.clone()).expect("link b.wasm");

        assert!(wasmfile.exists(), "wasm file created");
    }

    #[test]
    fn compile_a_and_b() {
        let tmp = TempDir::new().expect("create temporary directory");

        let mut linker = Link::new(&[test_file("a.c"), test_file("b.c")]);
        linker.cflag("-nostartfiles");
        linker.link_opt(LinkOpt::NoDefaultEntryPoint);

        let wasmfile = tmp.path().join("ab.wasm");

        linker.link(wasmfile.clone()).expect("link ab.wasm");

        assert!(wasmfile.exists(), "wasm file created");
    }

    #[test]
    fn compile_to_lucet() {
        let tmp = TempDir::new().expect("create temporary directory");

        let mut lucetc = Lucetc::new(&[test_file("a.c"), test_file("b.c")]);
        lucetc.cflag("-nostartfiles");
        lucetc.link_opt(LinkOpt::NoDefaultEntryPoint);

        let so_file = tmp.path().join("ab.so");

        lucetc.build(&so_file).expect("compile ab.so");

        assert!(so_file.exists(), "so file created");
    }
}
