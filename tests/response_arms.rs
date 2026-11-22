//! The check of issue #238: a fallback (`_`, a binding, `else`) on a coordinator's `Response`
//! names `Response::Error` too, or says why one line above: `// response-fallback: <why>`.

use syn::punctuated::Punctuated;
use syn::spanned::Spanned;
use syn::visit::{self, Visit};
use syn::{Attribute, Expr, Pat, Token};

const MARKER: &str = "// response-fallback:";

/// What the patterns of one site name.
#[derive(Default)]
struct Names {
    response: bool,
    error: bool,
    ok_response: bool,
    err: bool,
}

/// How much one pattern takes: nothing more than it names, every `Response`, or every value.
#[derive(Clone, Copy, PartialEq, PartialOrd)]
enum Takes {
    Named,
    EveryResponse,
    Everything,
}

/// A pattern under a guard names nothing for the rule: its arm takes only some values.
fn read_path(path: &syn::Path, guarded: bool, names: &mut Names) {
    let mut segments = path.segments.iter().map(|s| s.ident.to_string());
    if segments.any(|s| s == "Response") {
        if let Some(variant) = segments.next() {
            names.response = true;
            names.error |= variant == "Error" && !guarded;
        }
    }
}

fn read_pat(pat: &Pat, guarded: bool, names: &mut Names) {
    match pat {
        Pat::Path(p) => read_path(&p.path, guarded, names),
        Pat::Struct(p) => {
            read_path(&p.path, guarded, names);
            p.fields
                .iter()
                .for_each(|f| read_pat(&f.pat, guarded, names));
        }
        Pat::TupleStruct(p) => {
            read_path(&p.path, guarded, names);
            let mut inside = Names::default();
            p.elems
                .iter()
                .for_each(|e| read_pat(e, guarded, &mut inside));
            names.ok_response |= inside.ok_response || (p.path.is_ident("Ok") && inside.response);
            names.err |= inside.err || (p.path.is_ident("Err") && !guarded);
            names.response |= inside.response;
            names.error |= inside.error;
        }
        Pat::Tuple(p) => p.elems.iter().for_each(|e| read_pat(e, guarded, names)),
        Pat::Or(p) => p.cases.iter().for_each(|e| read_pat(e, guarded, names)),
        Pat::Paren(p) => read_pat(&p.pat, guarded, names),
        Pat::Reference(p) => read_pat(&p.pat, guarded, names),
        Pat::Guard(p) => read_pat(&p.pat, true, names),
        Pat::Ident(p) => {
            if let Some((_, sub)) = &p.subpat {
                read_pat(sub, guarded, names);
            }
        }
        _ => {}
    }
}

fn takes(pat: &Pat) -> Takes {
    match pat {
        Pat::Wild(_) => Takes::Everything,
        Pat::Ident(p) => match &p.subpat {
            Some((_, sub)) => takes(sub),
            // `None` and a constant are idents too; a binding starts with a small letter.
            None if is_a_binding(&p.ident.to_string()) => Takes::Everything,
            None => Takes::Named,
        },
        Pat::Paren(p) => takes(&p.pat),
        Pat::Reference(p) => takes(&p.pat),
        Pat::Guard(p) => takes(&p.pat),
        Pat::Or(p) => p
            .cases
            .iter()
            .map(takes)
            .fold(Takes::Named, |a, b| if b > a { b } else { a }),
        Pat::TupleStruct(p)
            if (p.path.is_ident("Ok") || p.path.is_ident("Some"))
                && p.elems.len() == 1
                && takes(&p.elems[0]) != Takes::Named =>
        {
            Takes::EveryResponse
        }
        _ => Takes::Named,
    }
}

fn is_a_binding(ident: &str) -> bool {
    ident.starts_with(|c: char| c.is_lowercase() || c == '_')
}

fn is_cfg_test(attrs: &[Attribute]) -> bool {
    attrs.iter().any(|a| {
        a.path().is_ident("cfg")
            && a.parse_args::<syn::Path>()
                .is_ok_and(|p| p.is_ident("test"))
    })
}

/// The first argument of `matches!` and its pattern.
struct MatchesBody(Expr, Pat);

impl syn::parse::Parse for MatchesBody {
    fn parse(input: syn::parse::ParseStream) -> syn::Result<Self> {
        let value: Expr = input.parse()?;
        input.parse::<Token![,]>()?;
        let pat = Pat::parse_multi_with_leading_vert(input)?;
        if input.peek(Token![if]) {
            input.parse::<Token![if]>()?;
            input.parse::<Expr>()?;
        }
        input.parse::<Option<Token![,]>>()?;
        Ok(MatchesBody(value, pat))
    }
}

struct Check<'a> {
    lines: Vec<&'a str>,
    sites: usize,
    used: Vec<usize>,
    found: Vec<(usize, String)>,
}

/// True when the tokens of a macro hold the word `Response`, at any depth.
fn holds_the_word_response(tokens: proc_macro2::TokenStream) -> bool {
    tokens.into_iter().any(|tree| match tree {
        proc_macro2::TokenTree::Ident(word) => word == "Response",
        proc_macro2::TokenTree::Group(group) => holds_the_word_response(group.stream()),
        _ => false,
    })
}

impl Check<'_> {
    /// Records one finding at `line`, unless the line above it says why the code is right.
    fn flag(&mut self, line: usize, text: String) {
        let above = line.checked_sub(2).and_then(|i| self.lines.get(i));
        match above.and_then(|text| text.trim().strip_prefix(MARKER)) {
            Some(why) if !why.trim().is_empty() => self.used.push(line - 1),
            Some(_) => {
                self.used.push(line - 1);
                self.found
                    .push((line - 1, "a marker without a reason".into()));
            }
            None => self.found.push((line, text)),
        }
    }

    /// One pattern with an `else`, or with nothing, for every value that it does not name.
    fn one_pattern(&mut self, pat: &Pat, line: usize, kind: &str) {
        let mut names = Names::default();
        read_pat(pat, false, &mut names);
        if !names.response {
            return;
        }
        self.sites += 1;
        let mut missing = Vec::new();
        if !names.error {
            missing.push("Response::Error");
        }
        if names.ok_response && !names.err {
            missing.push("Err(..)");
        }
        if !missing.is_empty() {
            self.flag(
                line,
                format!("{kind} does not name {}", missing.join(" or ")),
            );
        }
    }

    fn let_in_a_condition(&mut self, cond: &Expr, line: usize, kind: &str) {
        match cond {
            Expr::Let(l) => self.one_pattern(&l.pat, line, kind),
            Expr::Binary(b) => {
                self.let_in_a_condition(&b.left, line, kind);
                self.let_in_a_condition(&b.right, line, kind);
            }
            Expr::Paren(p) => self.let_in_a_condition(&p.expr, line, kind),
            _ => {}
        }
    }
}

impl<'ast> Visit<'ast> for Check<'_> {
    fn visit_item_mod(&mut self, node: &'ast syn::ItemMod) {
        if !is_cfg_test(&node.attrs) {
            visit::visit_item_mod(self, node);
        }
    }

    fn visit_item_fn(&mut self, node: &'ast syn::ItemFn) {
        if !is_cfg_test(&node.attrs) {
            visit::visit_item_fn(self, node);
        }
    }

    fn visit_expr_match(&mut self, node: &'ast syn::ExprMatch) {
        let mut names = Names::default();
        // The first arm that takes every `Response`, and the first that takes an `Err` too.
        let (mut first, mut first_wide) = (None, None);
        for arm in &node.arms {
            read_pat(&arm.pat, false, &mut names);
            let arm_takes = takes(&arm.pat);
            let line = arm.span().start().line;
            if arm_takes != Takes::Named && first.is_none() {
                first = Some(line);
            }
            if arm_takes == Takes::Everything && first_wide.is_none() {
                first_wide = Some(line);
            }
        }
        if names.response {
            self.sites += 1;
            let no_err = names.ok_response && !names.err;
            let kind = "the fallback arm of this match does not name";
            match first {
                Some(line) if !names.error && no_err && first_wide == first => {
                    self.flag(line, format!("{kind} Response::Error or Err(..)"));
                }
                Some(line) => {
                    if !names.error {
                        self.flag(line, format!("{kind} Response::Error"));
                    }
                    if let Some(wide) = first_wide.filter(|_| no_err) {
                        self.flag(wide, format!("{kind} Err(..)"));
                    }
                }
                None => {}
            }
        }
        visit::visit_expr_match(self, node);
    }

    fn visit_local(&mut self, node: &'ast syn::Local) {
        if node.init.as_ref().is_some_and(|i| i.diverge.is_some()) {
            self.one_pattern(&node.pat, node.let_token.span.start().line, "this let-else");
        }
        visit::visit_local(self, node);
    }

    fn visit_expr_if(&mut self, node: &'ast syn::ExprIf) {
        let kind = match node.else_branch {
            Some(_) => "this if-let with an else",
            None => "this if-let with no else",
        };
        self.let_in_a_condition(&node.cond, node.if_token.span.start().line, kind);
        visit::visit_expr_if(self, node);
    }

    fn visit_expr_while(&mut self, node: &'ast syn::ExprWhile) {
        let line = node.while_token.span.start().line;
        self.let_in_a_condition(&node.cond, line, "this while-let");
        visit::visit_expr_while(self, node);
    }

    fn visit_macro(&mut self, node: &'ast syn::Macro) {
        let line = node.path.span().start().line;
        if node.path.is_ident("matches") {
            if let Ok(MatchesBody(value, pat)) = node.parse_body::<MatchesBody>() {
                self.one_pattern(&pat, line, "this matches!");
                self.visit_expr(&value);
                return;
            }
        }
        match node.parse_body_with(Punctuated::<Expr, Token![,]>::parse_terminated) {
            Ok(arguments) => arguments.iter().for_each(|e| self.visit_expr(e)),
            // No lenient path: a body that names an answer and that syn cannot read is a finding.
            Err(_) if holds_the_word_response(node.tokens.clone()) => {
                let text = "syn cannot read the body of this macro, which names a Response";
                self.flag(line, text.into());
            }
            Err(_) => {}
        }
    }
}

/// Checks one source text. Gives the count of sites that read a `Response`, and one
/// `(line, text)` for each finding; an `Err` when the text is no Rust that syn can read.
fn check_source(text: &str) -> Result<(usize, Vec<(usize, String)>), String> {
    let file = syn::parse_file(text).map_err(|e| {
        let at = e.span().start();
        format!("{}:{}: syn cannot read this file: {e}", at.line, at.column)
    })?;
    let mut check = Check {
        lines: text.lines().collect(),
        sites: 0,
        used: Vec::new(),
        found: Vec::new(),
    };
    check.visit_file(&file);
    for (index, line) in check.lines.iter().enumerate() {
        if line.trim().starts_with(MARKER) && !check.used.contains(&(index + 1)) {
            let text = "a marker that is not directly above a fallback it excuses";
            check.found.push((index + 1, text.into()));
        }
    }
    check.found.sort();
    Ok((check.sites, check.found))
}

/// One row: its name, a source text, and each finding that the check must give for it.
type Row = (&'static str, &'static str, &'static [(usize, &'static str)]);

const MATCH_ERROR: &str = "the fallback arm of this match does not name Response::Error";
const MATCH_BOTH: &str = "the fallback arm of this match does not name Response::Error or Err(..)";
const MATCH_ERR: &str = "the fallback arm of this match does not name Err(..)";
const LET_ELSE: &str = "this let-else does not name Response::Error";
const NO_REASON: &str = "a marker without a reason";
const NO_USE: &str = "a marker that is not directly above a fallback it excuses";
const NO_RUST: &str = "syn cannot read this text";

/// The shapes that the fixes of the eight issues of the Gotcha removed.
const HISTORIC: &[Row] = &[
    (
        "#205 and #212: a let-else that bails with its own words",
        r#"fn gc(client: &mut Client) -> Result<i32> {
    let Response::Jobs { jobs } = client.call(&Request::List)? else {
        bail!("the coordinator did not give the job list");
    };
    Ok(0)
}"#,
        &[(2, LET_ELSE)],
    ),
    (
        "#212: a match with no arm for the error",
        r#"fn read_the_queue(client: &mut Client) -> Result<Jobs> {
    match client.call(&Request::Query { filter })? {
        Response::Found { jobs, older, .. } => Ok((jobs, older)),
        _ => bail!("the coordinator did not give the job list"),
    }
}"#,
        &[(4, MATCH_ERROR)],
    ),
    (
        "#218: every fault of a Result gives None",
        r#"fn jobs_running(client: &mut Client) -> Option<usize> {
    match client.call(&Request::Info) {
        Ok(Response::Info { jobs_running, .. }) => Some(jobs_running),
        _ => None,
    }
}"#,
        &[(4, MATCH_BOTH)],
    ),
    (
        "#221, #222 and #234: an Err under a guard, then a default",
        r#"fn coordinator_capabilities(client: &mut Client) -> Result<(String, i32)> {
    let (version, pid) = match client.call(&Request::Info) {
        Ok(Response::Info { version, pid, .. }) => (version, pid),
        Err(e) if is_a_coordinator_timeout(&e) => return Err(e),
        _ => return Ok((String::from("unknown"), 0)),
    };
    Ok((version, pid))
}"#,
        &[(5, MATCH_BOTH)],
    ),
    (
        "#234: an empty list for every other answer",
        r#"impl Client {
    fn can(&mut self, name: &str) -> Result<bool> {
        let names = match self.call(&Request::Capabilities)? {
            Response::Capabilities { names } => names,
            _ => Vec::new(),
        };
        Ok(names.iter().any(|n| n == name))
    }
}"#,
        &[(5, MATCH_ERROR)],
    ),
];

/// The shapes that those fixes wrote, which pass.
const FIXED: &[Row] = &[
    (
        "an arm for the error, then a fallback that bails",
        r#"fn job_list(client: &mut Client) -> Result<Vec<JobStatus>> {
    match client.call(&Request::List)? {
        Response::Jobs { jobs } => Ok(jobs),
        Response::Error { message, .. } => bail!("{message}"),
        _ => bail!("the coordinator did not give the job list"),
    }
}"#,
        &[],
    ),
    (
        "a fallback under a guard before the arm for the error",
        r#"fn capabilities(client: &mut Client, below: bool) -> Result<Vec<String>> {
    match client.call(&Request::Capabilities)? {
        Response::Capabilities { names } => Ok(names),
        _ if below => Ok(Vec::new()),
        Response::Error { message, .. } => bail!("{message}"),
        _ => bail!("the coordinator did not say what it can do"),
    }
}"#,
        &[],
    ),
    (
        "a Result with an arm for the error and an arm for every Err",
        r#"fn version(client: &mut Client) -> Result<Option<String>> {
    Ok(match client.call(&Request::Info) {
        Ok(Response::Info { version, .. }) => Some(version),
        Ok(Response::Error { message, .. }) => bail!("{message}"),
        Ok(_) => bail!("the coordinator did not give its version"),
        Err(e) if is_a_coordinator_timeout(&e) => return Err(e),
        Err(_) => None,
    })
}"#,
        &[],
    ),
    (
        "a pattern that reads the error alone",
        r#"fn page(info: &Option<Response>) {
    if let Some(Response::Error { message, .. }) = info {
        eprintln!("fairq: {message}");
    }
    let Response::Error { message, .. } = answer else {
        return;
    };
}"#,
        &[],
    ),
];

/// The edges of the rule.
const RULE: &[Row] = &[
    (
        "a binding is a fallback, in a match and inside Ok",
        r#"fn f(client: &mut Client) -> Result<i32> {
    match client.call(&Request::Cancel { id })? {
        Response::Ok => Ok(0),
        other => report(other),
    }
    match client.call(&Request::Status { id }) {
        Ok(Response::Status { status }) => Some(Some(status)),
        Ok(_answer) => Some(None),
        Err(_) => None,
    }
}"#,
        &[(4, MATCH_ERROR), (8, MATCH_ERROR)],
    ),
    (
        "an Err under a guard leaves the fallback with the other faults",
        r#"fn version(client: &mut Client) -> Result<Option<String>> {
    Ok(match client.call(&Request::Info) {
        Ok(Response::Info { version, .. }) => Some(version),
        Ok(Response::Error { message, .. }) => bail!("{message}"),
        Ok(_) => bail!("the coordinator did not give its version"),
        Err(e) if is_a_coordinator_timeout(&e) => return Err(e),
        _ => None,
    })
}"#,
        &[(7, MATCH_ERR)],
    ),
    (
        "an error under a guard is not the arm for the error",
        r#"fn f(answer: Response) -> i32 {
    match answer {
        Response::Ok => 0,
        Response::Error { kind, .. } if kind == ErrorKind::NoSuchJob => 127,
        _ => 0,
    }
}"#,
        &[(5, MATCH_ERROR)],
    ),
    (
        "an if-let, with an else and with none, and a while-let",
        r#"fn f(client: &mut Client) -> Result<bool> {
    let mut all_stopped = true;
    if let Response::Status { status } = client.call(&Request::Status { id })? {
        all_stopped = status.state.is_terminal();
    }
    if let Ok(Response::Status { status }) = client.call(&Request::Status { id }) {
        return Ok(true);
    } else {
        all_stopped = false;
    }
    while let Response::Event { event } = client.next()? {
        show(event);
    }
    Ok(all_stopped)
}"#,
        &[
            (3, "this if-let with no else does not name Response::Error"),
            (
                6,
                "this if-let with an else does not name Response::Error or Err(..)",
            ),
            (11, "this while-let does not name Response::Error"),
        ],
    ),
    (
        "a match inside an arm of a match that passes, and an arm of two lines",
        r#"fn f(client: &mut Client) -> Result<Option<JobStatus>> {
    match client.call(&Request::Resolve { handles })? {
        Response::Resolved { found } => {
            Ok(match client.call(&Request::Status { id: found[0] })? {
                Response::Status { status } => Some(*status),
                Response::Ok
                | _ => None,
            })
        }
        Response::Error { message, .. } => bail!("{message}"),
        _ => bail!("the coordinator did not read the ids"),
    }
}"#,
        &[(6, MATCH_ERROR)],
    ),
    (
        "a later fallback in a function whose first match passes",
        r#"fn f(client: &mut Client) -> Result<usize> {
    let jobs = match client.call(&Request::List)? {
        Response::Jobs { jobs } => jobs,
        Response::Error { message, .. } => bail!("{message}"),
        _ => bail!("the coordinator did not give the job list"),
    };
    let Response::Info { jobs_running, .. } = client.call(&Request::Info)? else {
        return Ok(0);
    };
    match client.call(&Request::Capabilities)? {
        Response::Capabilities { names } => Ok(names.len()),
        _ => Ok(0),
    }
}"#,
        &[(7, LET_ELSE), (12, MATCH_ERROR)],
    ),
    (
        "matches!, a match in the arguments of a macro, and a macro that syn cannot read",
        r#"fn f(answer: Response) {
    if matches!(answer, Response::Ok) {
        return;
    }
    assert!(matches!(answer, Response::Error { .. }));
    println!("{}", match answer {
        Response::Ok => 1,
        _ => 0,
    });
    let rows = table!(Response::Ok => 1; _ => 0);
    let other = table!(a => 1; _ => 0);
}"#,
        &[
            (2, "this matches! does not name Response::Error"),
            (8, MATCH_ERROR),
            (
                10,
                "syn cannot read the body of this macro, which names a Response",
            ),
        ],
    ),
    (
        "the code of tests is left alone, and the code after it is not",
        r#"#[cfg(test)]
mod tests {
    fn t() {
        let Response::Ok = answer() else { panic!() };
    }
}
#[cfg(not(test))]
fn g() {
    let Response::Ok = answer() else { return };
}
fn h() {
    let Response::Ok = answer() else { return };
}"#,
        &[(9, LET_ELSE), (12, LET_ELSE)],
    ),
    (
        "a path with its module, a reference, and code that reads no Response",
        r#"fn f(answer: proto::Response, kind: ErrorKind, count: Option<u32>) -> Response {
    match answer {
        proto::Response::Ok => {}
        _ => {}
    }
    match &answer {
        &Response::Ok => {}
        (&_) => {}
    }
    match answer {
        crate::proto::Response::Ok => {}
        crate::proto::Response::Error { .. } => {}
        _ => {}
    }
    match info {
        Some(Response::Info { .. }) => {}
        None => {}
    }
    match kind {
        ErrorKind::NoSuchJob => {}
        _ => {}
    }
    let Some(n) = count else {
        return Response::error(ErrorKind::Internal, "no count");
    };
    Response::Ok
}"#,
        &[(4, MATCH_ERROR), (8, MATCH_ERROR)],
    ),
];

/// The marker, and a text that is no Rust.
const MARKERS: &[Row] = &[
    (
        "a marker with a reason excuses a match, a let-else and an if-let",
        r#"fn f(client: &mut Client) -> Result<i32> {
    match client.call(&Request::Cancel { id })? {
        Response::Ok => {}
        // response-fallback: report names the error
        other => return report(other),
    }
    // response-fallback: a courtesy read; the next question reports the fault
    let Ok(Response::PauseState { queue, .. }) = client.call(&Request::PauseState) else {
        return Ok(0);
    };
    // response-fallback: #197 counts a refused record as kept and says no reason
    if let Response::Ok = client.call(&Request::Clean { id })? {
        deleted += 1;
    }
    Ok(0)
}"#,
        &[],
    ),
    (
        "a marker without a reason is refused",
        r#"fn f(client: &mut Client) -> Result<i32> {
    match client.call(&Request::Cancel { id })? {
        Response::Ok => Ok(0),
        // response-fallback:
        other => report(other),
    }
}"#,
        &[(4, NO_REASON)],
    ),
    (
        "a marker two lines above excuses nothing",
        r#"fn f(client: &mut Client) -> Result<i32> {
    match client.call(&Request::Cancel { id })? {
        Response::Ok => Ok(0),
        // response-fallback: report names the error
        // Every other answer is a fault.
        other => report(other),
    }
}"#,
        &[(4, NO_USE), (6, MATCH_ERROR)],
    ),
    (
        "a marker above a fallback that needs none is refused",
        r#"fn f(client: &mut Client) -> Result<i32> {
    match client.call(&Request::Cancel { id })? {
        Response::Ok => Ok(0),
        Response::Error { message, .. } => bail!("{message}"),
        // response-fallback: no longer needed
        _ => bail!("the coordinator gave another answer"),
    }
}"#,
        &[(5, NO_USE)],
    ),
    (
        "a Result needs one marker for each arm that the rule names",
        r#"fn f(client: &mut Client) -> Result<Option<String>> {
    Ok(match client.call(&Request::Info) {
        Ok(Response::Info { version, .. }) => Some(version),
        // response-fallback: an answer of another kind has no version
        Ok(_) => None,
        Err(e) if is_a_coordinator_timeout(&e) => return Err(e),
        _ => None,
    })
}"#,
        &[(7, MATCH_ERR)],
    ),
    (
        "a text that is no Rust is a finding, and never a pass",
        r#"fn f(answer: Response) -> i32 {
    match answer {
        Response::Ok => 0,
        _ => 1,
"#,
        &[(0, NO_RUST)],
    ),
];

/// The check, row by row. A row that fails prints `FAIL` with what the check gave.
#[test]
fn each_fixture_row_gets_its_verdict() {
    let mut failed = Vec::new();
    for (name, source, expected) in [HISTORIC, FIXED, RULE, MARKERS].concat() {
        let (sites, got) = match check_source(source) {
            Ok(checked) => checked,
            Err(_) => (1, vec![(0, NO_RUST.to_string())]),
        };
        // A row with no finding passes only when the check saw a site that reads a Response.
        let same = sites > 0
            && got.len() == expected.len()
            && got
                .iter()
                .zip(expected)
                .all(|(g, e)| g.0 == e.0 && g.1 == e.1);
        if same {
            println!("row ok: {name}");
        } else {
            println!("row FAIL: {name}\n    the check saw {sites} sites and gave {got:?}");
            failed.push(name);
        }
    }
    assert!(
        failed.is_empty(),
        "{} rows failed, listed above",
        failed.len()
    );
}
