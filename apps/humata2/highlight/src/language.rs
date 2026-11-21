//! Language detection by file name, extension, shebang, modeline or Markdown
//! fence info string.

/// Every programming, markup and configuration language Humata 2 detects and
/// highlights.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Language {
    Rust,
    Go,
    Python,
    Toml,
    Yaml,
    Json,
    Java,
    Kotlin,
    C,
    Cpp,
    Swift,
    TypeScript,
    Tsx,
    JavaScript,
    Jsx,
    Shell,
    Sql,
    Terraform,
    Markdown,
    Dockerfile,
    Makefile,
    Protobuf,
    Starlark,
    Html,
    Css,
    Scss,
    Ruby,
    Php,
    CSharp,
    Scala,
    Lua,
    Zig,
    GraphQL,
    Diff,
    CMake,
    #[default]
    Plain,
}

impl Language {
    /// Detects the language from an optional path and optional file content:
    /// file name first, then extension, then shebang or editor modeline.
    #[must_use]
    pub fn detect(path: Option<&str>, content: Option<&str>) -> Self {
        if let Some(path) = path {
            let from_path = Self::from_path(path);
            if from_path != Self::Plain {
                return from_path;
            }
        }
        if let Some(content) = content {
            return Self::from_content(content);
        }
        Self::Plain
    }

    /// Detects the language from a file path: exact file name first, then
    /// extension. Dots in directory names are ignored.
    #[must_use]
    pub fn from_path(path: &str) -> Self {
        let name = path.rsplit(['/', '\\']).next().unwrap_or(path);
        if name.is_empty() {
            return Self::Plain;
        }
        let by_name = Self::from_filename(name);
        if by_name != Self::Plain {
            return by_name;
        }
        let Some((_, ext)) = name.rsplit_once('.') else {
            return Self::Plain;
        };
        if ext.is_empty() {
            return Self::Plain;
        }
        Self::from_extension(ext)
    }

    /// Detects the language from a file's content using a leading shebang
    /// (`#!...`) or a Vim/Emacs modeline in the first or last 5 lines.
    #[must_use]
    pub fn from_content(content: &str) -> Self {
        let mut lines = content.lines();
        if let Some(first) = lines.next() {
            if let Some(lang) = Self::from_shebang(first) {
                return lang;
            }
            if let Some(lang) = Self::from_modeline(first) {
                return lang;
            }
        }
        for line in lines.take(4) {
            if let Some(lang) = Self::from_modeline(line) {
                return lang;
            }
        }
        for line in content.lines().rev().take(5) {
            if let Some(lang) = Self::from_modeline(line) {
                return lang;
            }
        }
        Self::Plain
    }

    /// Detects the language from a Markdown code fence info string (such as
    /// `"rust"`, `"ts"`, `"bzl"`, `"hcl"`, `"Dockerfile"` or `"BUILD.bazel"`).
    #[must_use]
    pub fn from_fence(info: &str) -> Self {
        let token = info
            .trim()
            .split(|c: char| c.is_whitespace() || c == ',' || c == '{')
            .next()
            .unwrap_or("")
            .trim_start_matches('.');
        if token.is_empty() {
            return Self::Plain;
        }
        let by_alias = Self::from_alias(token);
        if by_alias != Self::Plain {
            return by_alias;
        }
        Self::from_path(token)
    }

    /// The token passed to the underlying syntect highlighter, or `None` for
    /// plain text.
    #[must_use]
    pub const fn syntax_token(self) -> Option<&'static str> {
        match self {
            Self::Rust => Some("rust"),
            Self::Go => Some("go"),
            Self::Python => Some("python"),
            Self::Toml => Some("toml"),
            Self::Yaml => Some("yaml"),
            Self::Json => Some("json"),
            Self::Java => Some("java"),
            Self::Kotlin => Some("kotlin"),
            Self::C => Some("c"),
            Self::Cpp => Some("cpp"),
            Self::Swift => Some("swift"),
            Self::TypeScript => Some("typescript"),
            Self::Tsx => Some("tsx"),
            Self::JavaScript => Some("javascript"),
            Self::Jsx => Some("jsx"),
            Self::Shell => Some("bash"),
            Self::Sql => Some("sql"),
            Self::Terraform => Some("terraform"),
            Self::Markdown => Some("markdown"),
            Self::Dockerfile => Some("dockerfile"),
            Self::Makefile => Some("make"),
            Self::Protobuf => Some("protobuf"),
            Self::Starlark => Some("starlark"),
            Self::Html => Some("html"),
            Self::Css => Some("css"),
            Self::Scss => Some("scss"),
            Self::Ruby => Some("ruby"),
            Self::Php => Some("php"),
            Self::CSharp => Some("csharp"),
            Self::Scala => Some("scala"),
            Self::Lua => Some("lua"),
            Self::Zig => Some("zig"),
            Self::GraphQL => Some("graphql"),
            Self::Diff => Some("diff"),
            Self::CMake => Some("cmake"),
            Self::Plain => None,
        }
    }

    /// A short human-readable name for the language.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Rust => "Rust",
            Self::Go => "Go",
            Self::Python => "Python",
            Self::Toml => "TOML",
            Self::Yaml => "YAML",
            Self::Json => "JSON",
            Self::Java => "Java",
            Self::Kotlin => "Kotlin",
            Self::C => "C",
            Self::Cpp => "C++",
            Self::Swift => "Swift",
            Self::TypeScript => "TypeScript",
            Self::Tsx => "TSX",
            Self::JavaScript => "JavaScript",
            Self::Jsx => "JSX",
            Self::Shell => "Shell",
            Self::Sql => "SQL",
            Self::Terraform => "Terraform",
            Self::Markdown => "Markdown",
            Self::Dockerfile => "Dockerfile",
            Self::Makefile => "Makefile",
            Self::Protobuf => "Protobuf",
            Self::Starlark => "Starlark",
            Self::Html => "HTML",
            Self::Css => "CSS",
            Self::Scss => "SCSS",
            Self::Ruby => "Ruby",
            Self::Php => "PHP",
            Self::CSharp => "C#",
            Self::Scala => "Scala",
            Self::Lua => "Lua",
            Self::Zig => "Zig",
            Self::GraphQL => "GraphQL",
            Self::Diff => "Diff",
            Self::CMake => "CMake",
            Self::Plain => "Plain text",
        }
    }

    fn from_filename(name: &str) -> Self {
        match name {
            "BUILD" | "BUILD.bazel" | "MODULE.bazel" | "WORKSPACE" | "WORKSPACE.bazel"
            | "WORKSPACE.bzlmod" | "BUCK" | "Tiltfile" => return Self::Starlark,
            "Dockerfile" | "Containerfile" => return Self::Dockerfile,
            "Makefile" | "GNUmakefile" | "BSDmakefile" | "makefile" => return Self::Makefile,
            "CMakeLists.txt" => return Self::CMake,
            "Cargo.lock" | "Pipfile" | "poetry.lock" => return Self::Toml,
            "Gemfile" | "Rakefile" | "Vagrantfile" | "Podfile" | "Fastfile" => return Self::Ruby,
            ".bashrc" | ".bash_profile" | ".bash_logout" | ".zshrc" | ".zprofile" | ".zshenv"
            | ".profile" | "bashrc" | "zshrc" => return Self::Shell,
            _ => {}
        }
        let lower = name.to_ascii_lowercase();
        if lower.starts_with("dockerfile.")
            || lower.ends_with(".dockerfile")
            || lower.starts_with("containerfile.")
            || lower.ends_with(".containerfile")
        {
            return Self::Dockerfile;
        }
        if lower.starts_with("makefile.") {
            return Self::Makefile;
        }
        if name.starts_with("BUILD.") {
            return Self::Starlark;
        }
        Self::Plain
    }

    fn from_extension(ext: &str) -> Self {
        match ext.to_ascii_lowercase().as_str() {
            "rs" => Self::Rust,
            "go" => Self::Go,
            "py" | "pyi" | "pyw" => Self::Python,
            "toml" => Self::Toml,
            "yaml" | "yml" => Self::Yaml,
            "json" | "jsonc" | "jsonl" | "geojson" => Self::Json,
            "java" => Self::Java,
            "kt" | "kts" => Self::Kotlin,
            "c" | "h" => Self::C,
            "cc" | "cpp" | "cxx" | "c++" | "hpp" | "hxx" | "hh" | "h++" | "ipp" | "tpp" => {
                Self::Cpp
            }
            "swift" => Self::Swift,
            "ts" | "mts" | "cts" => Self::TypeScript,
            "tsx" => Self::Tsx,
            "js" | "mjs" | "cjs" => Self::JavaScript,
            "jsx" => Self::Jsx,
            "sh" | "bash" | "zsh" | "ksh" | "dash" | "ash" => Self::Shell,
            "sql" | "ddl" | "dml" => Self::Sql,
            "tf" | "tfvars" | "hcl" | "nomad" => Self::Terraform,
            "md" | "markdown" | "mdx" | "mkd" => Self::Markdown,
            "dockerfile" => Self::Dockerfile,
            "mk" | "mak" | "make" => Self::Makefile,
            "proto" => Self::Protobuf,
            "bzl" | "star" | "starlark" | "bazel" | "sky" => Self::Starlark,
            "html" | "htm" | "xhtml" => Self::Html,
            "css" => Self::Css,
            "scss" | "sass" => Self::Scss,
            "rb" | "gemspec" | "podspec" => Self::Ruby,
            "php" | "php3" | "php4" | "php5" | "phtml" => Self::Php,
            "cs" => Self::CSharp,
            "scala" | "sc" => Self::Scala,
            "lua" => Self::Lua,
            "zig" => Self::Zig,
            "graphql" | "gql" => Self::GraphQL,
            "diff" | "patch" => Self::Diff,
            "cmake" => Self::CMake,
            _ => Self::Plain,
        }
    }

    fn from_alias(alias: &str) -> Self {
        match alias.to_ascii_lowercase().as_str() {
            "rust" | "rs" => Self::Rust,
            "go" | "golang" => Self::Go,
            "python" | "python3" | "python2" | "py" | "py3" => Self::Python,
            "toml" => Self::Toml,
            "yaml" | "yml" => Self::Yaml,
            "json" | "jsonc" | "jsonl" => Self::Json,
            "java" => Self::Java,
            "kotlin" | "kt" | "kts" => Self::Kotlin,
            "c" => Self::C,
            "cpp" | "c++" | "cxx" | "cc" => Self::Cpp,
            "swift" => Self::Swift,
            "typescript" | "ts" => Self::TypeScript,
            "tsx" => Self::Tsx,
            "javascript" | "js" | "node" | "nodejs" | "deno" | "bun" => Self::JavaScript,
            "jsx" => Self::Jsx,
            "sh" | "bash" | "zsh" | "shell" | "shellscript" | "console" | "ksh" | "dash"
            | "ash" | "posix" => Self::Shell,
            "sql" | "postgres" | "postgresql" | "mysql" | "sqlite" | "tsql" | "plsql" => Self::Sql,
            "terraform" | "tf" | "hcl" | "tfvars" => Self::Terraform,
            "markdown" | "md" | "mdx" | "gfm" => Self::Markdown,
            "dockerfile" | "docker" | "containerfile" => Self::Dockerfile,
            "makefile" | "make" | "mk" => Self::Makefile,
            "protobuf" | "proto" | "proto2" | "proto3" => Self::Protobuf,
            "starlark" | "bzl" | "bazel" | "star" | "skylark" => Self::Starlark,
            "html" | "htm" => Self::Html,
            "css" => Self::Css,
            "scss" | "sass" => Self::Scss,
            "ruby" | "rb" => Self::Ruby,
            "php" => Self::Php,
            "csharp" | "c#" | "cs" => Self::CSharp,
            "scala" => Self::Scala,
            "lua" => Self::Lua,
            "zig" => Self::Zig,
            "graphql" | "gql" => Self::GraphQL,
            "diff" | "patch" => Self::Diff,
            "cmake" => Self::CMake,
            _ => Self::Plain,
        }
    }

    fn from_shebang(first_line: &str) -> Option<Self> {
        let rest = first_line.strip_prefix("#!")?.trim();
        let mut parts = rest.split_whitespace();
        let first = parts.next()?;
        let program = first.rsplit('/').next().unwrap_or(first);
        let interpreter = if program == "env" {
            // Skip flags like `-S` or `VAR=value` assignments before the interpreter.
            parts
                .find(|part| !part.starts_with('-') && !part.contains('='))?
                .rsplit('/')
                .next()
                .unwrap_or("")
        } else {
            program
        };
        let normalized = strip_version_suffix(interpreter);
        let lang = Self::from_alias(normalized);
        (lang != Self::Plain).then_some(lang)
    }

    fn from_modeline(line: &str) -> Option<Self> {
        // Emacs: `-*- mode: python -*-` or `-*- starlark -*-`
        if let Some(start) = line.find("-*-") {
            let after = &line[start + 3..];
            if let Some(end) = after.find("-*-") {
                let inner = after[..end].trim();
                for part in inner.split(';') {
                    let part = part.trim();
                    let candidate = if let Some((k, v)) = part.split_once(':') {
                        if k.trim().eq_ignore_ascii_case("mode") {
                            v.trim()
                        } else {
                            continue;
                        }
                    } else {
                        part
                    };
                    let candidate = candidate
                        .strip_suffix("-mode")
                        .or_else(|| candidate.strip_suffix("-ts-mode"))
                        .unwrap_or(candidate);
                    let lang = Self::from_alias(candidate);
                    if lang != Self::Plain {
                        return Some(lang);
                    }
                }
            }
        }
        // Vim: `vim: set ft=yaml:` / `vi: filetype=python` / `ex: syntax=rust`
        let lower = line.to_ascii_lowercase();
        let has_vim = lower.contains("vim:") || lower.contains("vi:") || lower.contains("ex:");
        if has_vim {
            for key in ["ft=", "filetype=", "syntax=", "syn="] {
                if let Some(pos) = lower.find(key) {
                    let rest = &line[pos + key.len()..];
                    let value = rest
                        .split(|c: char| c.is_whitespace() || c == ':' || c == ';')
                        .next()
                        .unwrap_or("");
                    let lang = Self::from_alias(value);
                    if lang != Self::Plain {
                        return Some(lang);
                    }
                }
            }
        }
        None
    }
}

/// Strips trailing version numbers such as `python3.12` -> `python`, `ruby3.3` -> `ruby`.
fn strip_version_suffix(name: &str) -> &str {
    let trimmed = name.trim_end_matches(|c: char| c.is_ascii_digit() || c == '.');
    if trimmed.is_empty() { name } else { trimmed }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn table_path_detection_covers_every_required_language() {
        let cases = [
            // Rust
            ("src/lib.rs", Language::Rust),
            ("SRC/MAIN.RS", Language::Rust),
            // Go
            ("cmd/server/main.go", Language::Go),
            // Python
            ("scripts/build.py", Language::Python),
            ("stubs/types.pyi", Language::Python),
            // TOML
            ("Cargo.toml", Language::Toml),
            ("Cargo.lock", Language::Toml),
            ("pyproject.toml", Language::Toml),
            // YAML
            (".github/workflows/ci.yml", Language::Yaml),
            ("config/app.yaml", Language::Yaml),
            // JSON
            ("package.json", Language::Json),
            ("tsconfig.jsonc", Language::Json),
            // Java
            ("src/main/java/App.java", Language::Java),
            // Kotlin
            ("app/src/Main.kt", Language::Kotlin),
            ("build.gradle.kts", Language::Kotlin),
            // C / C++
            ("src/util.c", Language::C),
            ("include/util.h", Language::C),
            ("src/engine.cpp", Language::Cpp),
            ("src/engine.cc", Language::Cpp),
            ("include/engine.hpp", Language::Cpp),
            // Swift
            ("Sources/App/ContentView.swift", Language::Swift),
            // TypeScript / JavaScript
            ("web/src/index.ts", Language::TypeScript),
            ("web/src/App.tsx", Language::Tsx),
            ("web/src/index.js", Language::JavaScript),
            ("web/src/util.mjs", Language::JavaScript),
            ("web/src/App.jsx", Language::Jsx),
            // Shell
            ("scripts/deploy.sh", Language::Shell),
            ("scripts/setup.bash", Language::Shell),
            ("scripts/env.zsh", Language::Shell),
            (".zshrc", Language::Shell),
            (".bashrc", Language::Shell),
            // SQL
            ("migrations/001_init.sql", Language::Sql),
            // Terraform / HCL
            ("infra/main.tf", Language::Terraform),
            ("infra/prod.tfvars", Language::Terraform),
            ("infra/terragrunt.hcl", Language::Terraform),
            // Markdown
            ("README.md", Language::Markdown),
            ("docs/guide.markdown", Language::Markdown),
            // Dockerfile
            ("Dockerfile", Language::Dockerfile),
            ("deploy/Dockerfile.prod", Language::Dockerfile),
            ("deploy/api.dockerfile", Language::Dockerfile),
            ("Containerfile", Language::Dockerfile),
            // Makefile
            ("Makefile", Language::Makefile),
            ("GNUmakefile", Language::Makefile),
            ("build/rules.mk", Language::Makefile),
            ("Makefile.local", Language::Makefile),
            // Protobuf
            ("api/proto/humata/v1/events.proto", Language::Protobuf),
            // Starlark
            ("BUILD", Language::Starlark),
            ("apps/humata2/BUILD.bazel", Language::Starlark),
            ("MODULE.bazel", Language::Starlark),
            ("WORKSPACE", Language::Starlark),
            ("WORKSPACE.bazel", Language::Starlark),
            ("bazel/defs.bzl", Language::Starlark),
            ("tools/config.star", Language::Starlark),
            ("third_party/BUILD.zlib", Language::Starlark),
            // Plain / edge cases
            ("README", Language::Plain),
            ("dir.rs/LICENSE", Language::Plain),
            ("notes.unknown", Language::Plain),
        ];

        for (path, expected) in cases {
            assert_eq!(
                Language::from_path(path),
                expected,
                "path `{path}` should detect as {expected:?}"
            );
        }
    }

    #[test]
    fn shebang_and_modeline_detection_handles_extensionless_files() {
        let cases = [
            ("#!/usr/bin/env python3\nprint('hi')\n", Language::Python),
            ("#!/usr/bin/python3.12\nprint('hi')\n", Language::Python),
            ("#!/bin/sh\necho hi\n", Language::Shell),
            ("#!/usr/bin/env bash\nset -euo pipefail\n", Language::Shell),
            ("#!/usr/bin/env -S zsh -e\necho hi\n", Language::Shell),
            ("#!/usr/bin/env node\nconsole.log(1)\n", Language::JavaScript),
            ("#!/usr/bin/env ruby\nputs 1\n", Language::Ruby),
            ("# vim: set ft=yaml:\nkey: value\n", Language::Yaml),
            ("// vi: filetype=rust\nfn main() {}\n", Language::Rust),
            ("# -*- mode: starlark -*-\nload('//a:b.bzl', 'x')\n", Language::Starlark),
            ("# -*- python -*-\nx = 1\n", Language::Python),
            ("plain text without markers\n", Language::Plain),
        ];

        for (content, expected) in cases {
            assert_eq!(
                Language::detect(Some("bin/script"), Some(content)),
                expected,
                "content `{content:?}` should detect as {expected:?}"
            );
        }
    }

    #[test]
    fn path_takes_precedence_over_content() {
        let content = "#!/usr/bin/env python3\n";
        assert_eq!(
            Language::detect(Some("script.sh"), Some(content)),
            Language::Shell
        );
    }

    #[test]
    fn markdown_fence_info_strings_resolve_to_languages() {
        for (fence, expected) in [
            ("rust", Language::Rust),
            ("rs", Language::Rust),
            ("go", Language::Go),
            ("golang", Language::Go),
            ("python", Language::Python),
            ("py", Language::Python),
            ("toml", Language::Toml),
            ("yaml", Language::Yaml),
            ("yml", Language::Yaml),
            ("json", Language::Json),
            ("java", Language::Java),
            ("kotlin", Language::Kotlin),
            ("kt", Language::Kotlin),
            ("c", Language::C),
            ("cpp", Language::Cpp),
            ("c++", Language::Cpp),
            ("swift", Language::Swift),
            ("ts", Language::TypeScript),
            ("typescript", Language::TypeScript),
            ("tsx", Language::Tsx),
            ("js", Language::JavaScript),
            ("jsx", Language::Jsx),
            ("sh", Language::Shell),
            ("bash", Language::Shell),
            ("zsh", Language::Shell),
            ("sql", Language::Sql),
            ("terraform", Language::Terraform),
            ("hcl", Language::Terraform),
            ("tf", Language::Terraform),
            ("markdown", Language::Markdown),
            ("md", Language::Markdown),
            ("dockerfile", Language::Dockerfile),
            ("makefile", Language::Makefile),
            ("make", Language::Makefile),
            ("proto", Language::Protobuf),
            ("protobuf", Language::Protobuf),
            ("starlark", Language::Starlark),
            ("bzl", Language::Starlark),
            ("bazel", Language::Starlark),
            ("BUILD.bazel", Language::Starlark),
            ("rust,ignore", Language::Rust),
            ("python title=\"demo.py\"", Language::Python),
        ] {
            assert_eq!(
                Language::from_fence(fence),
                expected,
                "fence `{fence}` should resolve to {expected:?}"
            );
        }
    }
}
