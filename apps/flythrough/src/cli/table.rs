//! A flag table: the rows a parser reads and a `--help` prints, so the two cannot disagree.
//!
//! A row names its flag (and any alias), the word its value takes, one line of help and what
//! it does to the target `T`. A refused row names flags a plan removed: the parser fails on
//! them with the row's message, and `--help` leaves them out. The fly-through's own flags
//! ([`crate::cli::flags`]) and its examples' are tables of this kind.

use std::fmt::Write as _;
use std::str::FromStr;

/// A row's reader of its value.
type ValueAct<T> = Box<dyn Fn(&mut T, &Val<'_>) -> Result<(), String>>;

/// What a row does when its flag is read.
enum Act<T> {
    /// A flag without a value.
    Switch(Box<dyn Fn(&mut T)>),
    /// A flag that takes the next argument.
    Value(ValueAct<T>),
    /// A removed flag: the message the parser fails with, given the flag as written.
    Refused(fn(&str) -> String),
}

/// One row of a table.
pub struct Flag<T> {
    names: &'static [&'static str],
    value: &'static str,
    help: &'static str,
    act: Act<T>,
}

impl<T> Flag<T> {
    /// A flag without a value: `names[0]` and its aliases.
    pub fn switch(
        names: &'static [&'static str],
        help: &'static str,
        act: impl Fn(&mut T) + 'static,
    ) -> Self {
        Flag {
            names,
            value: "",
            help,
            act: Act::Switch(Box::new(act)),
        }
    }

    /// A flag taking one value, written `value` in `--help`.
    pub fn value(
        names: &'static [&'static str],
        value: &'static str,
        help: &'static str,
        act: impl Fn(&mut T, &Val<'_>) -> Result<(), String> + 'static,
    ) -> Self {
        Flag {
            names,
            value,
            help,
            act: Act::Value(Box::new(act)),
        }
    }

    /// Flags a plan removed: each is refused with `why(flag)` and none is listed by `--help`.
    pub fn refused(names: &'static [&'static str], why: fn(&str) -> String) -> Self {
        Flag {
            names,
            value: "",
            help: "",
            act: Act::Refused(why),
        }
    }

    /// Whether the parser refuses this row's flags.
    pub fn is_refused(&self) -> bool {
        matches!(self.act, Act::Refused(_))
    }

    /// The row's flag and its aliases.
    pub fn names(&self) -> &'static [&'static str] {
        self.names
    }
}

/// A titled group of rows; `--help` prints them under the title.
pub struct Section<T> {
    pub title: &'static str,
    pub flags: Vec<Flag<T>>,
}

/// A program's flags: its usage line and its sections.
pub struct Table<T> {
    pub usage: &'static str,
    pub sections: Vec<Section<T>>,
}

/// The value of a flag being read, with the shared readers every row uses. Each error names
/// the flag as written and the value.
pub struct Val<'a> {
    pub flag: &'a str,
    pub text: &'a str,
}

impl Val<'_> {
    /// The value as an owned string.
    pub fn string(&self) -> String {
        self.text.to_owned()
    }

    /// `on` or `off`.
    pub fn on_off(&self) -> Result<bool, String> {
        match self.text {
            "on" => Ok(true),
            "off" => Ok(false),
            v => Err(format!("{} takes on or off, not {v}", self.flag)),
        }
    }

    /// One of `words`, as its paired value; the error lists the words as `takes`.
    pub fn word<V: Copy>(&self, words: &[(&str, V)], takes: &str) -> Result<V, String> {
        words
            .iter()
            .find(|(w, _)| *w == self.text)
            .map(|&(_, v)| v)
            .ok_or_else(|| self.takes(takes))
    }

    /// A value an enum reads by name (`from_name`), or the error `takes`.
    pub fn named<V>(
        &self,
        from_name: impl Fn(&str) -> Option<V>,
        takes: &str,
    ) -> Result<V, String> {
        from_name(self.text).ok_or_else(|| self.takes(takes))
    }

    /// The value parsed as `V`, the parser's error after the flag (`--seed: invalid digit`).
    pub fn parse<V: FromStr>(&self) -> Result<V, String>
    where
        V::Err: std::fmt::Display,
    {
        self.text.parse().map_err(|e| format!("{}: {e}", self.flag))
    }

    /// The value parsed as `V` and kept by `ok`, or the error `takes`.
    pub fn parse_if<V: FromStr>(&self, ok: impl Fn(&V) -> bool, takes: &str) -> Result<V, String> {
        self.text
            .parse()
            .ok()
            .filter(|v| ok(v))
            .ok_or_else(|| self.takes(takes))
    }

    /// `<flag> takes <what>, not <value>`.
    pub fn takes(&self, what: &str) -> String {
        format!("{} takes {what}, not {}", self.flag, self.text)
    }
}

impl<T> Table<T> {
    fn row(&self, flag: &str) -> Option<&Flag<T>> {
        self.sections
            .iter()
            .flat_map(|s| &s.flags)
            .find(|f| f.names.contains(&flag))
    }

    /// Reads `argv` into `target`: each flag is looked up in the rows, a refused one fails
    /// with its row's message, one that takes a value reads the next argument, and an
    /// unknown one fails as `unknown argument <flag>`.
    pub fn parse_into(
        &self,
        argv: impl IntoIterator<Item = String>,
        target: &mut T,
    ) -> Result<(), String> {
        let mut it = argv.into_iter();
        while let Some(flag) = it.next() {
            let row = self
                .row(&flag)
                .ok_or_else(|| format!("unknown argument {flag}"))?;
            match &row.act {
                Act::Switch(f) => f(target),
                Act::Value(f) => {
                    let text = it.next().ok_or(format!("{flag} needs a value"))?;
                    f(
                        target,
                        &Val {
                            flag: &flag,
                            text: &text,
                        },
                    )?;
                }
                Act::Refused(why) => return Err(why(&flag)),
            }
        }
        Ok(())
    }

    /// Every flag the parser accepts (refused rows left out), aliases included, in table order.
    pub fn accepted(&self) -> Vec<&'static str> {
        self.sections
            .iter()
            .flat_map(|s| &s.flags)
            .filter(|f| !f.is_refused())
            .flat_map(|f| f.names.iter().copied())
            .collect()
    }

    /// The `--help` text: the usage line, then each section's rows, one a line, as
    /// `  --flag, --alias VALUE  help`.
    pub fn help(&self) -> String {
        let head = |f: &Flag<T>| {
            let names = f.names.join(", ");
            if f.value.is_empty() {
                names
            } else {
                format!("{names} {}", f.value)
            }
        };
        let width = self
            .sections
            .iter()
            .flat_map(|s| &s.flags)
            .filter(|f| !f.is_refused())
            .map(|f| head(f).len())
            .max()
            .unwrap_or(0);
        let mut out = format!("{}\n", self.usage);
        for s in self
            .sections
            .iter()
            .filter(|s| s.flags.iter().any(|f| !f.is_refused()))
        {
            let _ = write!(out, "\n{}:\n", s.title);
            for f in s.flags.iter().filter(|f| !f.is_refused()) {
                let _ = writeln!(out, "  {:width$}  {}", head(f), f.help);
            }
        }
        out
    }
}

/// The flags `help` lists: each row's first word that starts with `--` (or `-h`), aliases
/// after a comma included.
pub fn listed_flags(help: &str) -> Vec<String> {
    let mut out = Vec::new();
    for line in help.lines().filter(|l| l.starts_with("  -")) {
        for part in line.trim_start().split(", ") {
            let word = part.split_whitespace().next().unwrap_or_default();
            out.push(word.to_owned());
            if part.contains(' ') {
                break;
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct T {
        a: bool,
        n: u32,
    }

    fn table() -> Table<T> {
        Table {
            usage: "t [FLAGS]",
            sections: vec![
                Section {
                    title: "One",
                    flags: vec![
                        Flag::switch(&["--a"], "sets a", |t: &mut T| t.a = true),
                        Flag::value(&["--n", "--m"], "N", "sets n", |t: &mut T, v| {
                            t.n = v.parse()?;
                            Ok(())
                        }),
                    ],
                },
                Section {
                    title: "Removed",
                    flags: vec![Flag::refused(&["--old"], |f| format!("{f} is gone"))],
                },
            ],
        }
    }

    /// A switch, a value under its alias, a refused flag, a missing value and an unknown
    /// flag each read as their row says; `--help` lists the accepted rows only.
    #[test]
    fn a_table_parses_and_lists_its_rows() {
        let t = table();
        let mut x = T::default();
        t.parse_into(["--a", "--m", "7"].map(String::from), &mut x)
            .unwrap();
        assert!(x.a);
        assert_eq!(x.n, 7);
        let mut x = T::default();
        assert_eq!(
            t.parse_into(["--old".to_owned()], &mut x).err().as_deref(),
            Some("--old is gone")
        );
        assert_eq!(
            t.parse_into(["--n".to_owned()], &mut x).err().as_deref(),
            Some("--n needs a value")
        );
        assert_eq!(
            t.parse_into(["--z".to_owned()], &mut x).err().as_deref(),
            Some("unknown argument --z")
        );
        assert_eq!(t.accepted(), ["--a", "--n", "--m"]);
        let help = t.help();
        assert_eq!(listed_flags(&help), ["--a", "--n", "--m"]);
        assert!(!help.contains("--old"), "{help}");
        assert!(!help.contains("Removed"), "{help}");
    }
}
