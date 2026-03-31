//! Built-in build rules for Catalyst

use std::path::PathBuf;

use crate::{Action, Label, Target};

/// Types of built-in rules
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RuleType {
    /// Compile Rust source to a binary
    RustBinary,
    /// Compile Rust source to a library (.rlib)
    RustLibrary,
    /// Run Rust tests
    RustTest,
    /// Compile C/C++ source to a binary
    CcBinary,
    /// Compile C/C++ source to a library
    CcLibrary,
    /// Run C/C++ tests
    CcTest,
    /// Generic rule with custom command
    Genrule,
    /// Group files together (no action, passthrough)
    Filegroup,
    /// Alias to another target
    Alias,
    /// Export files from package
    ExportsFiles,
}

impl RuleType {
    /// Parse a rule type from its name
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "rust_binary" => Some(Self::RustBinary),
            "rust_library" => Some(Self::RustLibrary),
            "rust_test" => Some(Self::RustTest),
            "cc_binary" => Some(Self::CcBinary),
            "cc_library" => Some(Self::CcLibrary),
            "cc_test" => Some(Self::CcTest),
            "genrule" => Some(Self::Genrule),
            "filegroup" => Some(Self::Filegroup),
            "alias" => Some(Self::Alias),
            "exports_files" => Some(Self::ExportsFiles),
            _ => None,
        }
    }

    /// Get the rule name
    pub fn name(&self) -> &'static str {
        match self {
            Self::RustBinary => "rust_binary",
            Self::RustLibrary => "rust_library",
            Self::RustTest => "rust_test",
            Self::CcBinary => "cc_binary",
            Self::CcLibrary => "cc_library",
            Self::CcTest => "cc_test",
            Self::Genrule => "genrule",
            Self::Filegroup => "filegroup",
            Self::Alias => "alias",
            Self::ExportsFiles => "exports_files",
        }
    }

    /// Check if this rule produces actions
    pub fn has_actions(&self) -> bool {
        !matches!(self, Self::Filegroup | Self::Alias | Self::ExportsFiles)
    }
}

/// Result of rule expansion
#[derive(Debug)]
pub struct RuleExpansion {
    /// Actions to execute
    pub actions: Vec<Action>,
    /// Output files produced
    pub outputs: Vec<PathBuf>,
}

impl RuleExpansion {
    /// Create an empty expansion (for passthrough rules)
    pub fn empty() -> Self {
        Self {
            actions: Vec::new(),
            outputs: Vec::new(),
        }
    }

    /// Create expansion with a single action
    pub fn single(action: Action, outputs: Vec<PathBuf>) -> Self {
        Self {
            actions: vec![action],
            outputs,
        }
    }
}

/// Trait for rule implementations
pub trait Rule {
    /// Expand a target into actions
    fn expand(&self, target: &Target, output_dir: &PathBuf) -> RuleExpansion;
}

/// Rust binary rule
pub struct RustBinaryRule;

impl Rule for RustBinaryRule {
    fn expand(&self, target: &Target, output_dir: &PathBuf) -> RuleExpansion {
        let output_path = output_dir.join(&target.label.name);

        let mut cmd = vec![
            "rustc".to_string(),
            "--crate-type=bin".to_string(),
            "-o".to_string(),
            output_path.to_string_lossy().into_owned(),
        ];

        // Add source files
        for src in &target.srcs {
            cmd.push(src.to_string_lossy().into_owned());
        }

        // Add dependency libraries
        for dep in &target.deps {
            let lib_name = format!("lib{}.rlib", dep.name);
            let lib_path = output_dir.join(&lib_name);
            cmd.push("--extern".to_string());
            cmd.push(format!(
                "{}={}",
                dep.name,
                lib_path.to_string_lossy()
            ));
        }

        let mut action = Action::new(cmd);
        for src in &target.srcs {
            action.add_input(src.clone());
        }
        action.add_output(output_path.clone());

        RuleExpansion::single(action, vec![output_path])
    }
}

/// Rust library rule
pub struct RustLibraryRule;

impl Rule for RustLibraryRule {
    fn expand(&self, target: &Target, output_dir: &PathBuf) -> RuleExpansion {
        let lib_name = format!("lib{}.rlib", target.label.name);
        let output_path = output_dir.join(&lib_name);

        let mut cmd = vec![
            "rustc".to_string(),
            "--crate-type=rlib".to_string(),
            "-o".to_string(),
            output_path.to_string_lossy().into_owned(),
        ];

        // Add source files
        for src in &target.srcs {
            cmd.push(src.to_string_lossy().into_owned());
        }

        // Add dependency libraries
        for dep in &target.deps {
            let dep_lib_name = format!("lib{}.rlib", dep.name);
            let lib_path = output_dir.join(&dep_lib_name);
            cmd.push("--extern".to_string());
            cmd.push(format!(
                "{}={}",
                dep.name,
                lib_path.to_string_lossy()
            ));
        }

        let mut action = Action::new(cmd);
        for src in &target.srcs {
            action.add_input(src.clone());
        }
        action.add_output(output_path.clone());

        RuleExpansion::single(action, vec![output_path])
    }
}

/// Genrule - custom command
pub struct GenruleRule;

impl Rule for GenruleRule {
    fn expand(&self, target: &Target, output_dir: &PathBuf) -> RuleExpansion {
        // Get cmd attribute
        let cmd_str = target
            .get_attr("cmd")
            .and_then(|v| v.as_str())
            .unwrap_or("");

        if cmd_str.is_empty() {
            return RuleExpansion::empty();
        }

        // Build output paths from declared outs
        let outputs: Vec<PathBuf> = target
            .outs
            .iter()
            .map(|o| output_dir.join(o))
            .collect();

        // Build command - run through shell
        // Replace $SRCS, $OUTS, $@, $<
        let mut expanded_cmd = cmd_str.to_string();

        // $SRCS = space-separated inputs
        let srcs_str: String = target
            .srcs
            .iter()
            .map(|s| s.to_string_lossy().into_owned())
            .collect::<Vec<_>>()
            .join(" ");
        expanded_cmd = expanded_cmd.replace("$SRCS", &srcs_str);
        expanded_cmd = expanded_cmd.replace("$(SRCS)", &srcs_str);

        // $< = first input
        if let Some(first_src) = target.srcs.first() {
            let first = first_src.to_string_lossy();
            expanded_cmd = expanded_cmd.replace("$<", &first);
        }

        // $OUTS = space-separated outputs
        let outs_str: String = outputs
            .iter()
            .map(|o| o.to_string_lossy().into_owned())
            .collect::<Vec<_>>()
            .join(" ");
        expanded_cmd = expanded_cmd.replace("$OUTS", &outs_str);
        expanded_cmd = expanded_cmd.replace("$(OUTS)", &outs_str);

        // $@ = first output
        if let Some(first_out) = outputs.first() {
            let first = first_out.to_string_lossy();
            expanded_cmd = expanded_cmd.replace("$@", &first);
            expanded_cmd = expanded_cmd.replace("$OUT", &first);
        }

        let cmd = vec!["sh".to_string(), "-c".to_string(), expanded_cmd];

        let mut action = Action::new(cmd);
        for src in &target.srcs {
            action.add_input(src.clone());
        }
        for out in &outputs {
            action.add_output(out.clone());
        }

        RuleExpansion::single(action, outputs)
    }
}

/// Filegroup - group files with no action
pub struct FilegroupRule;

impl Rule for FilegroupRule {
    fn expand(&self, _target: &Target, _output_dir: &PathBuf) -> RuleExpansion {
        // Filegroup produces no actions - it's a passthrough
        RuleExpansion::empty()
    }
}

/// Get a rule implementation for a rule type
pub fn get_rule(rule_type: RuleType) -> Box<dyn Rule> {
    match rule_type {
        RuleType::RustBinary => Box::new(RustBinaryRule),
        RuleType::RustLibrary => Box::new(RustLibraryRule),
        RuleType::Genrule => Box::new(GenruleRule),
        RuleType::Filegroup => Box::new(FilegroupRule),
        // Default to empty for unimplemented rules
        _ => Box::new(FilegroupRule),
    }
}

/// Expand a target into actions using its rule
pub fn expand_target(target: &Target, output_dir: &PathBuf) -> RuleExpansion {
    let rule_type = RuleType::from_name(&target.rule_type).unwrap_or(RuleType::Filegroup);
    let rule = get_rule(rule_type);
    rule.expand(target, output_dir)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_target(name: &str, rule_type: &str) -> Target {
        Target::new(Label::new("pkg", name), rule_type)
    }

    #[test]
    fn rust_binary_generates_rustc_command() {
        let mut target = test_target("myapp", "rust_binary");
        target.add_src(PathBuf::from("src/main.rs"));

        let output_dir = PathBuf::from("/out");
        let expansion = expand_target(&target, &output_dir);

        assert_eq!(expansion.actions.len(), 1);
        let cmd = &expansion.actions[0].command;
        assert_eq!(cmd[0], "rustc");
        assert!(cmd.contains(&"--crate-type=bin".to_string()));
        assert!(cmd.contains(&"-o".to_string()));
        assert!(cmd.iter().any(|s| s.contains("myapp")));
        assert!(cmd.contains(&"src/main.rs".to_string()));
    }

    #[test]
    fn rust_library_generates_rlib_output() {
        let mut target = test_target("mylib", "rust_library");
        target.add_src(PathBuf::from("src/lib.rs"));

        let output_dir = PathBuf::from("/out");
        let expansion = expand_target(&target, &output_dir);

        assert_eq!(expansion.actions.len(), 1);
        assert_eq!(expansion.outputs.len(), 1);

        let output = &expansion.outputs[0];
        assert!(output.to_string_lossy().contains("libmylib.rlib"));

        let cmd = &expansion.actions[0].command;
        assert!(cmd.contains(&"--crate-type=rlib".to_string()));
    }

    #[test]
    fn rust_binary_includes_deps() {
        let mut target = test_target("myapp", "rust_binary");
        target.add_src(PathBuf::from("src/main.rs"));
        target.add_dep(Label::new("pkg", "mylib"));

        let output_dir = PathBuf::from("/out");
        let expansion = expand_target(&target, &output_dir);

        let cmd = &expansion.actions[0].command;
        assert!(cmd.contains(&"--extern".to_string()));
        assert!(cmd.iter().any(|s| s.contains("mylib")));
    }

    #[test]
    fn genrule_uses_provided_cmd() {
        let mut target = test_target("gen", "genrule");
        target.add_src(PathBuf::from("input.txt"));
        target.add_out(PathBuf::from("output.txt"));
        target.set_attr("cmd", crate::Value::String("cat $< > $@".to_string()));

        let output_dir = PathBuf::from("/out");
        let expansion = expand_target(&target, &output_dir);

        assert_eq!(expansion.actions.len(), 1);
        let cmd = &expansion.actions[0].command;
        assert_eq!(cmd[0], "sh");
        assert_eq!(cmd[1], "-c");
        assert!(cmd[2].contains("cat"));
        assert!(cmd[2].contains("input.txt"));
        assert!(cmd[2].contains("/out/output.txt"));
    }

    #[test]
    fn filegroup_has_no_actions() {
        let mut target = test_target("files", "filegroup");
        target.add_src(PathBuf::from("a.txt"));
        target.add_src(PathBuf::from("b.txt"));

        let output_dir = PathBuf::from("/out");
        let expansion = expand_target(&target, &output_dir);

        assert!(expansion.actions.is_empty());
    }

    #[test]
    fn rule_sets_inputs_and_outputs() {
        let mut target = test_target("app", "rust_binary");
        target.add_src(PathBuf::from("a.rs"));
        target.add_src(PathBuf::from("b.rs"));

        let output_dir = PathBuf::from("/out");
        let expansion = expand_target(&target, &output_dir);

        let action = &expansion.actions[0];
        assert_eq!(action.inputs.len(), 2);
        assert_eq!(action.outputs.len(), 1);
    }

    #[test]
    fn rule_type_from_name() {
        assert_eq!(
            RuleType::from_name("rust_binary"),
            Some(RuleType::RustBinary)
        );
        assert_eq!(
            RuleType::from_name("rust_library"),
            Some(RuleType::RustLibrary)
        );
        assert_eq!(RuleType::from_name("genrule"), Some(RuleType::Genrule));
        assert_eq!(RuleType::from_name("filegroup"), Some(RuleType::Filegroup));
        assert_eq!(RuleType::from_name("unknown"), None);
    }

    #[test]
    fn rule_type_has_actions() {
        assert!(RuleType::RustBinary.has_actions());
        assert!(RuleType::RustLibrary.has_actions());
        assert!(RuleType::Genrule.has_actions());
        assert!(!RuleType::Filegroup.has_actions());
        assert!(!RuleType::Alias.has_actions());
    }

    #[test]
    fn genrule_substitutes_srcs() {
        let mut target = test_target("gen", "genrule");
        target.add_src(PathBuf::from("a.txt"));
        target.add_src(PathBuf::from("b.txt"));
        target.add_out(PathBuf::from("out.txt"));
        target.set_attr("cmd", crate::Value::String("cat $SRCS > $@".to_string()));

        let output_dir = PathBuf::from("/out");
        let expansion = expand_target(&target, &output_dir);

        let cmd = &expansion.actions[0].command[2];
        assert!(cmd.contains("a.txt"));
        assert!(cmd.contains("b.txt"));
    }
}
