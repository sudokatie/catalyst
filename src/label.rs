//! Target label parsing and resolution

use std::fmt;
use std::str::FromStr;

use crate::Error;

/// A target label identifying a buildable target.
///
/// Labels have the form `//package:name` or `:name` (relative).
/// If only package is given (e.g., `//foo/bar`), name defaults to the package basename.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Label {
    /// Package path (e.g., "foo/bar")
    pub package: String,
    /// Target name within the package
    pub name: String,
}

impl Label {
    /// Create a new label
    pub fn new(package: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            package: package.into(),
            name: name.into(),
        }
    }

    /// Parse a label string.
    ///
    /// Formats:
    /// - `//package:name` - absolute label
    /// - `//package` - absolute, name = basename
    /// - `:name` - relative label (package empty)
    pub fn parse(s: &str) -> Result<Self, Error> {
        let s = s.trim();

        if s.is_empty() {
            return Err(Error::InvalidLabel("empty label".to_string()));
        }

        // Relative label
        if let Some(name) = s.strip_prefix(':') {
            if name.is_empty() {
                return Err(Error::InvalidLabel("empty target name".to_string()));
            }
            return Ok(Label {
                package: String::new(),
                name: name.to_string(),
            });
        }

        // Absolute label
        if let Some(rest) = s.strip_prefix("//") {
            if let Some((package, name)) = rest.split_once(':') {
                if name.is_empty() {
                    return Err(Error::InvalidLabel("empty target name".to_string()));
                }
                return Ok(Label {
                    package: package.to_string(),
                    name: name.to_string(),
                });
            }
            // No colon - name defaults to package basename
            let name = rest
                .rsplit('/')
                .next()
                .filter(|s| !s.is_empty())
                .ok_or_else(|| Error::InvalidLabel("empty package".to_string()))?;
            return Ok(Label {
                package: rest.to_string(),
                name: name.to_string(),
            });
        }

        Err(Error::InvalidLabel(format!(
            "label must start with '//' or ':': {s}"
        )))
    }

    /// Returns the absolute label string
    pub fn absolute(&self) -> String {
        format!("//{}:{}", self.package, self.name)
    }

    /// Returns true if this is a relative label (no package)
    pub fn is_relative(&self) -> bool {
        self.package.is_empty()
    }

    /// Resolve a relative label against a current package.
    ///
    /// If this label is already absolute, returns a clone.
    /// If relative, combines with the given package.
    pub fn resolve(&self, current_package: &str) -> Label {
        if self.is_relative() {
            Label {
                package: current_package.to_string(),
                name: self.name.clone(),
            }
        } else {
            self.clone()
        }
    }
}

impl fmt::Display for Label {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.is_relative() {
            write!(f, ":{}", self.name)
        } else {
            write!(f, "//{}:{}", self.package, self.name)
        }
    }
}

impl FromStr for Label {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Label::parse(s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_absolute_label() {
        let label = Label::parse("//foo/bar:baz").unwrap();
        assert_eq!(label.package, "foo/bar");
        assert_eq!(label.name, "baz");
        assert!(!label.is_relative());
    }

    #[test]
    fn parse_relative_label() {
        let label = Label::parse(":target").unwrap();
        assert_eq!(label.package, "");
        assert_eq!(label.name, "target");
        assert!(label.is_relative());
    }

    #[test]
    fn parse_package_only() {
        let label = Label::parse("//foo/bar").unwrap();
        assert_eq!(label.package, "foo/bar");
        assert_eq!(label.name, "bar");
    }

    #[test]
    fn parse_root_package() {
        let label = Label::parse("//myapp:main").unwrap();
        assert_eq!(label.package, "myapp");
        assert_eq!(label.name, "main");
    }

    #[test]
    fn resolve_relative() {
        let label = Label::parse(":mylib").unwrap();
        let resolved = label.resolve("pkg/sub");
        assert_eq!(resolved.package, "pkg/sub");
        assert_eq!(resolved.name, "mylib");
        assert!(!resolved.is_relative());
    }

    #[test]
    fn resolve_absolute_unchanged() {
        let label = Label::parse("//other:thing").unwrap();
        let resolved = label.resolve("pkg/sub");
        assert_eq!(resolved.package, "other");
        assert_eq!(resolved.name, "thing");
    }

    #[test]
    fn absolute_string() {
        let label = Label::new("foo/bar", "baz");
        assert_eq!(label.absolute(), "//foo/bar:baz");
    }

    #[test]
    fn display() {
        let abs = Label::new("pkg", "target");
        assert_eq!(format!("{abs}"), "//pkg:target");

        let rel = Label::parse(":rel").unwrap();
        assert_eq!(format!("{rel}"), ":rel");
    }

    #[test]
    fn from_str() {
        let label: Label = "//a/b:c".parse().unwrap();
        assert_eq!(label.package, "a/b");
        assert_eq!(label.name, "c");
    }

    #[test]
    fn invalid_labels() {
        assert!(Label::parse("").is_err());
        assert!(Label::parse(":").is_err());
        assert!(Label::parse("//pkg:").is_err());
        assert!(Label::parse("no_prefix").is_err());
    }
}
