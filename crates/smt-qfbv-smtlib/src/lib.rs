//! Shared QF_BV/Bool SMT-LIB frontend utilities.
//!
//! This crate is intentionally solver-agnostic.  It owns the yaspar-based
//! S-expression parser and common frontend policy/sink types; solver crates
//! lower parsed commands through their own adapters.

use std::{collections::HashMap, fmt};

use dashu::{float::DBig, integer::UBig};
use yaspar::{
    action::{
        ActionOnAttribute, ActionOnConstant, ActionOnIdentifier, ActionOnIndex, ActionOnSort,
        ActionOnString, ActionOnTerm, ParsingAction, ParsingResult, Pattern,
    },
    ast::{DatatypeDec, DatatypeDef, FunctionDef, Keyword},
    position::Range,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SExpr {
    Atom(String),
    List(Vec<SExpr>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SmtSort {
    Bool,
    Bv(u32),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AcceptedLogics {
    QfBvOnly,
    QfBvOrAll,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IncrementalPolicy {
    RejectPushPop,
    SimulatePushPop,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GetValuePolicy {
    SymbolsOnly,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FrontendOptions {
    pub accepted_logics: AcceptedLogics,
    pub incremental_policy: IncrementalPolicy,
    pub get_value_policy: GetValuePolicy,
    pub require_check_sat: bool,
    pub max_macro_expansion_depth: usize,
}

impl FrontendOptions {
    pub fn server_text() -> Self {
        Self {
            accepted_logics: AcceptedLogics::QfBvOnly,
            incremental_policy: IncrementalPolicy::RejectPushPop,
            get_value_policy: GetValuePolicy::SymbolsOnly,
            require_check_sat: true,
            max_macro_expansion_depth: 64,
        }
    }

    pub fn standalone() -> Self {
        Self {
            accepted_logics: AcceptedLogics::QfBvOrAll,
            incremental_policy: IncrementalPolicy::SimulatePushPop,
            get_value_policy: GetValuePolicy::SymbolsOnly,
            require_check_sat: true,
            max_macro_expansion_depth: 64,
        }
    }
}

impl Default for FrontendOptions {
    fn default() -> Self {
        Self::server_text()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FrontendError {
    Parse(String),
    Unsupported(String),
    Invalid {
        context: &'static str,
        message: String,
    },
    Sink(String),
}

impl FrontendError {
    pub fn invalid(context: &'static str, message: impl Into<String>) -> Self {
        Self::Invalid {
            context,
            message: message.into(),
        }
    }
}

impl fmt::Display for FrontendError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FrontendError::Parse(message) => write!(f, "SMT-LIB parse error: {message}"),
            FrontendError::Unsupported(message) => {
                write!(f, "unsupported SMT-LIB construct: {message}")
            }
            FrontendError::Invalid { context, message } => {
                write!(f, "invalid {context}: {message}")
            }
            FrontendError::Sink(message) => write!(f, "frontend sink error: {message}"),
        }
    }
}

impl std::error::Error for FrontendError {}

pub type Result<T> = std::result::Result<T, FrontendError>;

pub trait QfBvSink {
    type Node: Copy;
    type Error: fmt::Display;

    fn bool_const(&mut self, value: bool) -> std::result::Result<Self::Node, Self::Error>;
    fn bv_const(
        &mut self,
        bytes_le: &[u8],
        width: u32,
    ) -> std::result::Result<Self::Node, Self::Error>;

    fn bool_var(&mut self, name: &str) -> std::result::Result<Self::Node, Self::Error>;
    fn bv_var(&mut self, name: &str, width: u32) -> std::result::Result<Self::Node, Self::Error>;

    fn bool_not(&mut self, x: Self::Node) -> std::result::Result<Self::Node, Self::Error>;
    fn bool_and(
        &mut self,
        a: Self::Node,
        b: Self::Node,
    ) -> std::result::Result<Self::Node, Self::Error>;
    fn bool_or(
        &mut self,
        a: Self::Node,
        b: Self::Node,
    ) -> std::result::Result<Self::Node, Self::Error>;
    fn bool_implies(
        &mut self,
        a: Self::Node,
        b: Self::Node,
    ) -> std::result::Result<Self::Node, Self::Error>;
    fn bool_eq(
        &mut self,
        a: Self::Node,
        b: Self::Node,
    ) -> std::result::Result<Self::Node, Self::Error>;
    fn bool_xor(
        &mut self,
        a: Self::Node,
        b: Self::Node,
    ) -> std::result::Result<Self::Node, Self::Error>;
    fn bool_ite(
        &mut self,
        c: Self::Node,
        t: Self::Node,
        e: Self::Node,
    ) -> std::result::Result<Self::Node, Self::Error>;

    fn bv_not(&mut self, x: Self::Node) -> std::result::Result<Self::Node, Self::Error>;
    fn bv_neg(&mut self, x: Self::Node) -> std::result::Result<Self::Node, Self::Error>;
    fn bv_and(
        &mut self,
        a: Self::Node,
        b: Self::Node,
    ) -> std::result::Result<Self::Node, Self::Error>;
    fn bv_or(
        &mut self,
        a: Self::Node,
        b: Self::Node,
    ) -> std::result::Result<Self::Node, Self::Error>;
    fn bv_xor(
        &mut self,
        a: Self::Node,
        b: Self::Node,
    ) -> std::result::Result<Self::Node, Self::Error>;
    fn bv_add(
        &mut self,
        a: Self::Node,
        b: Self::Node,
    ) -> std::result::Result<Self::Node, Self::Error>;
    fn bv_sub(
        &mut self,
        a: Self::Node,
        b: Self::Node,
    ) -> std::result::Result<Self::Node, Self::Error>;
    fn bv_mul(
        &mut self,
        a: Self::Node,
        b: Self::Node,
    ) -> std::result::Result<Self::Node, Self::Error>;
    fn bv_udiv(
        &mut self,
        a: Self::Node,
        b: Self::Node,
    ) -> std::result::Result<Self::Node, Self::Error>;
    fn bv_urem(
        &mut self,
        a: Self::Node,
        b: Self::Node,
    ) -> std::result::Result<Self::Node, Self::Error>;
    fn bv_sdiv(
        &mut self,
        a: Self::Node,
        b: Self::Node,
    ) -> std::result::Result<Self::Node, Self::Error>;
    fn bv_srem(
        &mut self,
        a: Self::Node,
        b: Self::Node,
    ) -> std::result::Result<Self::Node, Self::Error>;
    fn bv_smod(
        &mut self,
        a: Self::Node,
        b: Self::Node,
    ) -> std::result::Result<Self::Node, Self::Error>;
    fn bv_shl(
        &mut self,
        a: Self::Node,
        b: Self::Node,
    ) -> std::result::Result<Self::Node, Self::Error>;
    fn bv_lshr(
        &mut self,
        a: Self::Node,
        b: Self::Node,
    ) -> std::result::Result<Self::Node, Self::Error>;
    fn bv_ashr(
        &mut self,
        a: Self::Node,
        b: Self::Node,
    ) -> std::result::Result<Self::Node, Self::Error>;

    fn bv_extract(
        &mut self,
        x: Self::Node,
        hi: u32,
        lo: u32,
    ) -> std::result::Result<Self::Node, Self::Error>;
    fn bv_concat(
        &mut self,
        high: Self::Node,
        low: Self::Node,
    ) -> std::result::Result<Self::Node, Self::Error>;
    fn bv_zext(
        &mut self,
        x: Self::Node,
        amount: u32,
    ) -> std::result::Result<Self::Node, Self::Error>;
    fn bv_sext(
        &mut self,
        x: Self::Node,
        amount: u32,
    ) -> std::result::Result<Self::Node, Self::Error>;
    fn bv_repeat(
        &mut self,
        x: Self::Node,
        count: u32,
    ) -> std::result::Result<Self::Node, Self::Error>;
    fn bv_rotate_left(
        &mut self,
        x: Self::Node,
        amount: u32,
    ) -> std::result::Result<Self::Node, Self::Error>;
    fn bv_rotate_right(
        &mut self,
        x: Self::Node,
        amount: u32,
    ) -> std::result::Result<Self::Node, Self::Error>;
    fn bv_ite(
        &mut self,
        c: Self::Node,
        t: Self::Node,
        e: Self::Node,
    ) -> std::result::Result<Self::Node, Self::Error>;

    fn bv_eq(
        &mut self,
        a: Self::Node,
        b: Self::Node,
    ) -> std::result::Result<Self::Node, Self::Error>;
    fn bv_ult(
        &mut self,
        a: Self::Node,
        b: Self::Node,
    ) -> std::result::Result<Self::Node, Self::Error>;
    fn bv_ule(
        &mut self,
        a: Self::Node,
        b: Self::Node,
    ) -> std::result::Result<Self::Node, Self::Error>;
    fn bv_slt(
        &mut self,
        a: Self::Node,
        b: Self::Node,
    ) -> std::result::Result<Self::Node, Self::Error>;
    fn bv_sle(
        &mut self,
        a: Self::Node,
        b: Self::Node,
    ) -> std::result::Result<Self::Node, Self::Error>;

    fn uadd_ovf(
        &mut self,
        a: Self::Node,
        b: Self::Node,
    ) -> std::result::Result<Self::Node, Self::Error>;
    fn sadd_ovf(
        &mut self,
        a: Self::Node,
        b: Self::Node,
    ) -> std::result::Result<Self::Node, Self::Error>;
    fn usub_ovf(
        &mut self,
        a: Self::Node,
        b: Self::Node,
    ) -> std::result::Result<Self::Node, Self::Error>;
    fn ssub_ovf(
        &mut self,
        a: Self::Node,
        b: Self::Node,
    ) -> std::result::Result<Self::Node, Self::Error>;
    fn umul_ovf(
        &mut self,
        a: Self::Node,
        b: Self::Node,
    ) -> std::result::Result<Self::Node, Self::Error>;
    fn smul_ovf(
        &mut self,
        a: Self::Node,
        b: Self::Node,
    ) -> std::result::Result<Self::Node, Self::Error>;
    fn neg_ovf(&mut self, x: Self::Node) -> std::result::Result<Self::Node, Self::Error>;
    fn sdiv_ovf(
        &mut self,
        a: Self::Node,
        b: Self::Node,
    ) -> std::result::Result<Self::Node, Self::Error>;

    fn assert(
        &mut self,
        root: Self::Node,
        name: Option<&str>,
    ) -> std::result::Result<(), Self::Error>;
    fn assume(&mut self, root: Self::Node) -> std::result::Result<(), Self::Error>;

    fn push(&mut self) -> std::result::Result<(), Self::Error>;
    fn pop(&mut self) -> std::result::Result<(), Self::Error>;
}

pub fn parse_sexprs(input: &str) -> Result<Vec<SExpr>> {
    let mut action = SExprAction;
    yaspar::smtlib2::ScriptParser::new()
        .parse(&mut action, yaspar::tokenize_str(input, true))
        .map_err(|err| FrontendError::Parse(err.to_string()))
}

pub fn atom(expr: &SExpr) -> Result<&str> {
    match expr {
        SExpr::Atom(atom) => Ok(atom),
        SExpr::List(_) => Err(FrontendError::invalid("SMT-LIB atom", "expected atom")),
    }
}

pub fn parse_u32_atom(expr: &SExpr, context: &'static str) -> Result<u32> {
    atom(expr)?
        .parse::<u32>()
        .map_err(|_| FrontendError::invalid(context, "expected u32 numeral"))
}

pub fn parse_function_params(expr: &SExpr) -> Result<Vec<(String, SmtSort)>> {
    let SExpr::List(args) = expr else {
        return Err(FrontendError::invalid(
            "define-fun",
            "expected argument list",
        ));
    };
    let mut params = Vec::with_capacity(args.len());
    for arg in args {
        let SExpr::List(pair) = arg else {
            return Err(FrontendError::invalid(
                "define-fun",
                "argument is not a pair",
            ));
        };
        expect_len(pair, 2, "define-fun")?;
        let name = atom(&pair[0])?.to_owned();
        if params.iter().any(|(existing, _)| existing == &name) {
            return Err(FrontendError::invalid(
                "define-fun",
                format!("duplicate argument {name:?}"),
            ));
        }
        params.push((name, parse_sort(&pair[1])?));
    }
    Ok(params)
}

pub fn parse_sort(expr: &SExpr) -> Result<SmtSort> {
    match expr {
        SExpr::Atom(atom) if atom == "Bool" => Ok(SmtSort::Bool),
        SExpr::List(items)
            if items.len() == 3 && atom(&items[0])? == "_" && atom(&items[1])? == "BitVec" =>
        {
            let width = atom(&items[2])?
                .parse::<u32>()
                .map_err(|_| FrontendError::invalid("sort", "invalid BitVec width"))?;
            validate_bv_width(width, "sort")?;
            Ok(SmtSort::Bv(width))
        }
        _ => Err(FrontendError::invalid(
            "sort",
            "expected Bool or (_ BitVec n)",
        )),
    }
}

pub fn validate_bv_width(width: u32, context: &'static str) -> Result<()> {
    if (1..=65_536).contains(&width) {
        Ok(())
    } else {
        Err(FrontendError::invalid(
            context,
            format!("invalid BitVec width {width}"),
        ))
    }
}

pub fn decimal_to_le_bytes(text: &str, len: usize) -> Result<Vec<u8>> {
    let mut digits = text
        .bytes()
        .map(|byte| match byte {
            b'0'..=b'9' => Ok(byte - b'0'),
            _ => Err(FrontendError::invalid(
                "bv literal",
                "invalid decimal value",
            )),
        })
        .collect::<Result<Vec<_>>>()?;
    let mut out = Vec::with_capacity(len);
    while !digits.is_empty() && out.len() < len {
        let mut carry = 0u16;
        let mut quotient = Vec::new();
        for digit in digits {
            let n = carry * 10 + u16::from(digit);
            let q = (n / 256) as u8;
            carry = n % 256;
            if !quotient.is_empty() || q != 0 {
                quotient.push(q);
            }
        }
        out.push(carry as u8);
        digits = quotient;
    }
    out.resize(len, 0);
    Ok(out)
}

pub fn indexed_bv_literal_bytes(items: &[SExpr]) -> Result<(Vec<u8>, u32)> {
    let literal = if items.len() == 4 && atom(&items[1])? == "bv" {
        Some((atom(&items[2])?, atom(&items[3])?))
    } else if items.len() == 3 {
        let symbol = atom(&items[1])?;
        if let Some(value) = symbol.strip_prefix("bv") {
            Some((value, atom(&items[2])?))
        } else {
            None
        }
    } else {
        None
    };
    let Some((value, width)) = literal else {
        return Err(FrontendError::invalid(
            "indexed literal",
            "expected (_ bvN W)",
        ));
    };
    if value.is_empty() {
        return Err(FrontendError::invalid("bv literal", "missing value"));
    }
    let width = width
        .parse::<u32>()
        .map_err(|_| FrontendError::invalid("bv literal", "invalid width"))?;
    validate_bv_width(width, "bv literal")?;
    let mut bytes = decimal_to_le_bytes(value, (width as usize).div_ceil(8))?;
    mask_unused_high_bits(width, &mut bytes);
    Ok((bytes, width))
}

pub fn literal_bytes(digits: &str, radix: u32, width: u32, original: &str) -> Result<Vec<u8>> {
    validate_bv_width(width, "bv literal")?;
    let mut bytes = vec![0u8; (width as usize).div_ceil(8)];
    for (pos, ch) in digits.chars().rev().enumerate() {
        let value = ch.to_digit(radix).ok_or_else(|| {
            FrontendError::invalid("bv literal", format!("invalid digit in {original}"))
        })?;
        if radix == 2 {
            if value != 0 {
                bytes[pos / 8] |= 1u8 << (pos % 8);
            }
        } else {
            bytes[pos / 2] |= (value as u8) << ((pos % 2) * 4);
        }
    }
    mask_unused_high_bits(width, &mut bytes);
    Ok(bytes)
}

pub fn parse_annotation(items: &[SExpr]) -> Result<Option<(&SExpr, Option<String>)>> {
    if items.is_empty() || atom(&items[0])? != "!" {
        return Ok(None);
    }
    if items.len() < 4 || !(items.len() - 2).is_multiple_of(2) {
        return Err(FrontendError::invalid(
            "annotation",
            "expected annotated term followed by keyword/value pairs",
        ));
    }
    let mut name = None;
    for pair in items[2..].as_chunks::<2>().0 {
        let key = atom(&pair[0])?;
        if !key.starts_with(':') {
            return Err(FrontendError::invalid(
                "annotation",
                "expected annotation keyword",
            ));
        }
        if key == ":named" {
            name = Some(atom(&pair[1])?.to_owned());
        }
    }
    Ok(Some((&items[1], name)))
}

pub fn named_annotation(expr: &SExpr) -> Result<Option<(&SExpr, String)>> {
    let SExpr::List(items) = expr else {
        return Ok(None);
    };
    let Some((inner, name)) = parse_annotation(items)? else {
        return Ok(None);
    };
    let Some(name) = name else {
        return Ok(None);
    };
    Ok(Some((inner, name)))
}

pub fn expect_len(items: &[SExpr], len: usize, context: &'static str) -> Result<()> {
    if items.len() != len {
        return Err(FrontendError::invalid(
            context,
            format!("expected {len} items"),
        ));
    }
    Ok(())
}

pub fn expect_sort(actual: SmtSort, expected: SmtSort, context: &'static str) -> Result<()> {
    if actual != expected {
        return Err(FrontendError::invalid(
            context,
            format!("expected {expected:?}, got {actual:?}"),
        ));
    }
    Ok(())
}

pub fn mask_unused_high_bits(width: u32, bytes: &mut [u8]) {
    let valid_bits = width % 8;
    if valid_bits != 0 {
        if let Some(last) = bytes.last_mut() {
            *last &= (1u8 << valid_bits) - 1;
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoweredScript {
    pub want_model: bool,
    pub want_core: bool,
    pub get_values: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LoweredExpr<N> {
    pub node: N,
    pub sort: SmtSort,
}

#[derive(Debug, Clone)]
struct FunctionBinding {
    params: Vec<(String, SmtSort)>,
    result: SmtSort,
    body: SExpr,
}

pub fn lower_script<S: QfBvSink>(
    script: &str,
    sink: &mut S,
    options: &FrontendOptions,
) -> Result<LoweredScript> {
    let exprs = parse_sexprs(script)?;
    let mut lowerer = ScriptLowerer::new(sink, options.clone());
    lowerer.lower_script(exprs)
}

fn sink_result<T, E: fmt::Display>(result: std::result::Result<T, E>) -> Result<T> {
    result.map_err(|err| FrontendError::Sink(err.to_string()))
}

type UnarySinkOp<S> = fn(
    &mut S,
    <S as QfBvSink>::Node,
) -> std::result::Result<<S as QfBvSink>::Node, <S as QfBvSink>::Error>;
type BinarySinkOp<S> = fn(
    &mut S,
    <S as QfBvSink>::Node,
    <S as QfBvSink>::Node,
) -> std::result::Result<<S as QfBvSink>::Node, <S as QfBvSink>::Error>;

struct ScriptLowerer<'a, S: QfBvSink> {
    sink: &'a mut S,
    options: FrontendOptions,
    env: HashMap<String, LoweredExpr<S::Node>>,
    declared_symbols: HashMap<String, SmtSort>,
    functions: HashMap<String, FunctionBinding>,
    expansion_depth: usize,
    want_model: bool,
    want_core: bool,
    get_values: Vec<String>,
    saw_check_sat: bool,
}

impl<'a, S: QfBvSink> ScriptLowerer<'a, S> {
    fn new(sink: &'a mut S, options: FrontendOptions) -> Self {
        Self {
            sink,
            options,
            env: HashMap::new(),
            declared_symbols: HashMap::new(),
            functions: HashMap::new(),
            expansion_depth: 0,
            want_model: false,
            want_core: false,
            get_values: Vec::new(),
            saw_check_sat: false,
        }
    }

    fn lower_script(&mut self, exprs: Vec<SExpr>) -> Result<LoweredScript> {
        for expr in exprs {
            let list = match expr {
                SExpr::List(list) => list,
                SExpr::Atom(atom) => {
                    return Err(FrontendError::invalid(
                        "SMT-LIB command",
                        format!("top-level atom {atom:?}"),
                    ))
                }
            };
            if list.is_empty() {
                continue;
            }
            self.command(&list)?;
        }
        if self.options.require_check_sat && !self.saw_check_sat {
            return Err(FrontendError::invalid(
                "SMT-LIB script",
                "script does not contain check-sat",
            ));
        }
        Ok(LoweredScript {
            want_model: self.want_model,
            want_core: self.want_core,
            get_values: std::mem::take(&mut self.get_values),
        })
    }

    fn command(&mut self, list: &[SExpr]) -> Result<()> {
        let cmd = atom(&list[0])?;
        match cmd {
            "set-logic" => self.set_logic(list),
            "set-option" | "set-info" | "echo" | "exit" => Ok(()),
            "push" => self.push_command(list),
            "pop" => self.pop_command(list),
            "reset" | "reset-assertions" => Err(FrontendError::Unsupported(format!(
                "command {cmd} is not supported"
            ))),
            "declare-const" => self.declare_const(list),
            "declare-fun" => self.declare_fun(list),
            "define-const" => self.define_const(list),
            "define-fun" => self.define_fun(list),
            "assert" => self.assert_command(list),
            "check-sat" => {
                expect_len(list, 1, "check-sat")?;
                self.saw_check_sat = true;
                Ok(())
            }
            "check-sat-assuming" => self.check_sat_assuming(list),
            "get-model" => {
                expect_len(list, 1, "get-model")?;
                self.want_model = true;
                Ok(())
            }
            "get-value" => self.get_value_command(list),
            "get-unsat-core" => {
                expect_len(list, 1, "get-unsat-core")?;
                self.want_core = true;
                Ok(())
            }
            other => Err(FrontendError::Unsupported(format!(
                "SMT-LIB command {other}"
            ))),
        }
    }

    fn set_logic(&self, list: &[SExpr]) -> Result<()> {
        expect_len(list, 2, "set-logic")?;
        let logic = atom(&list[1])?;
        let accepted = match self.options.accepted_logics {
            AcceptedLogics::QfBvOnly => logic == "QF_BV",
            AcceptedLogics::QfBvOrAll => logic == "QF_BV" || logic == "ALL",
        };
        if accepted {
            Ok(())
        } else {
            Err(FrontendError::invalid(
                "set-logic",
                format!("unsupported logic {logic:?}"),
            ))
        }
    }

    fn command_level(&self, list: &[SExpr], context: &'static str) -> Result<u32> {
        if list.len() == 1 {
            return Ok(1);
        }
        if list.len() != 2 {
            return Err(FrontendError::invalid(context, "expected optional level"));
        }
        parse_u32_atom(&list[1], context)
    }

    fn push_command(&mut self, list: &[SExpr]) -> Result<()> {
        let levels = self.command_level(list, "push")?;
        match self.options.incremental_policy {
            IncrementalPolicy::RejectPushPop => Err(FrontendError::invalid(
                "SMT-LIB incremental command",
                "push is not supported on this frontend",
            )),
            IncrementalPolicy::SimulatePushPop => {
                for _ in 0..levels {
                    sink_result(self.sink.push())?;
                }
                Ok(())
            }
        }
    }

    fn pop_command(&mut self, list: &[SExpr]) -> Result<()> {
        let levels = self.command_level(list, "pop")?;
        match self.options.incremental_policy {
            IncrementalPolicy::RejectPushPop => Err(FrontendError::invalid(
                "SMT-LIB incremental command",
                "pop is not supported on this frontend",
            )),
            IncrementalPolicy::SimulatePushPop => {
                for _ in 0..levels {
                    sink_result(self.sink.pop())?;
                }
                Ok(())
            }
        }
    }

    fn declare_const(&mut self, list: &[SExpr]) -> Result<()> {
        expect_len(list, 3, "declare-const")?;
        let name = atom(&list[1])?.to_owned();
        let sort = parse_sort(&list[2])?;
        let node = match sort {
            SmtSort::Bool => sink_result(self.sink.bool_var(&name))?,
            SmtSort::Bv(width) => sink_result(self.sink.bv_var(&name, width))?,
        };
        self.env.insert(name.clone(), LoweredExpr { node, sort });
        self.declared_symbols.insert(name, sort);
        Ok(())
    }

    fn declare_fun(&mut self, list: &[SExpr]) -> Result<()> {
        expect_len(list, 4, "declare-fun")?;
        match &list[2] {
            SExpr::List(args) if args.is_empty() => {}
            _ => {
                return Err(FrontendError::Unsupported(
                    "only 0-arity declare-fun is supported".to_owned(),
                ))
            }
        }
        self.declare_const(&[list[0].clone(), list[1].clone(), list[3].clone()])
    }

    fn define_const(&mut self, list: &[SExpr]) -> Result<()> {
        expect_len(list, 4, "define-const")?;
        let name = atom(&list[1])?.to_owned();
        let declared = parse_sort(&list[2])?;
        let binding = self.expr_with_locals(&list[3], &mut HashMap::new())?;
        expect_sort(binding.sort, declared, "define-const")?;
        self.env.insert(name, binding);
        Ok(())
    }

    fn define_fun(&mut self, list: &[SExpr]) -> Result<()> {
        expect_len(list, 5, "define-fun")?;
        let name = atom(&list[1])?.to_owned();
        let params = parse_function_params(&list[2])?;
        if params.is_empty() {
            return self.define_const(&[
                list[0].clone(),
                list[1].clone(),
                list[3].clone(),
                list[4].clone(),
            ]);
        }
        let result = parse_sort(&list[3])?;
        self.functions.insert(
            name,
            FunctionBinding {
                params,
                result,
                body: list[4].clone(),
            },
        );
        Ok(())
    }

    fn assert_command(&mut self, list: &[SExpr]) -> Result<()> {
        expect_len(list, 2, "assert")?;
        let (expr, name) = match named_annotation(&list[1])? {
            Some((inner, name)) => (inner, Some(name)),
            None => (&list[1], None),
        };
        let binding = self.expr_with_locals(expr, &mut HashMap::new())?;
        expect_sort(binding.sort, SmtSort::Bool, "assert")?;
        sink_result(self.sink.assert(binding.node, name.as_deref()))
    }

    fn check_sat_assuming(&mut self, list: &[SExpr]) -> Result<()> {
        expect_len(list, 2, "check-sat-assuming")?;
        let assumptions = match &list[1] {
            SExpr::List(items) => items,
            _ => {
                return Err(FrontendError::invalid(
                    "check-sat-assuming",
                    "expected assumption list",
                ))
            }
        };
        for item in assumptions {
            let binding = self.expr_with_locals(item, &mut HashMap::new())?;
            expect_sort(binding.sort, SmtSort::Bool, "check-sat-assuming")?;
            sink_result(self.sink.assume(binding.node))?;
        }
        self.saw_check_sat = true;
        Ok(())
    }

    fn get_value_command(&mut self, list: &[SExpr]) -> Result<()> {
        expect_len(list, 2, "get-value")?;
        let terms = match &list[1] {
            SExpr::List(items) => items,
            _ => return Err(FrontendError::invalid("get-value", "expected term list")),
        };
        self.want_model = true;
        for term in terms {
            match (self.options.get_value_policy, term) {
                (GetValuePolicy::SymbolsOnly, SExpr::Atom(name)) => {
                    if !self.declared_symbols.contains_key(name) {
                        return Err(FrontendError::invalid(
                            "get-value",
                            format!("unknown declared symbol {name:?}"),
                        ));
                    }
                    let binding = self.env.get(name).copied().ok_or_else(|| {
                        FrontendError::invalid("get-value", format!("undefined symbol {name:?}"))
                    })?;
                    let tautology = self.equality_node(binding, binding, "get-value")?;
                    sink_result(self.sink.assert(tautology, None))?;
                    self.get_values.push(name.clone());
                }
                (GetValuePolicy::SymbolsOnly, _) => {
                    return Err(FrontendError::Unsupported(
                        "get-value currently supports declared symbols".to_owned(),
                    ))
                }
            }
        }
        Ok(())
    }

    fn expr_with_locals(
        &mut self,
        expr: &SExpr,
        locals: &mut HashMap<String, LoweredExpr<S::Node>>,
    ) -> Result<LoweredExpr<S::Node>> {
        match expr {
            SExpr::Atom(value) => self.atom_expr(value, locals),
            SExpr::List(items) => self.list_expr(items, locals),
        }
    }

    fn atom_expr(
        &mut self,
        value: &str,
        locals: &HashMap<String, LoweredExpr<S::Node>>,
    ) -> Result<LoweredExpr<S::Node>> {
        if value == "true" {
            return Ok(LoweredExpr {
                node: sink_result(self.sink.bool_const(true))?,
                sort: SmtSort::Bool,
            });
        }
        if value == "false" {
            return Ok(LoweredExpr {
                node: sink_result(self.sink.bool_const(false))?,
                sort: SmtSort::Bool,
            });
        }
        if let Some((digits, radix)) = value
            .strip_prefix("#b")
            .map(|s| (s, 2))
            .or_else(|| value.strip_prefix("#x").map(|s| (s, 16)))
        {
            let width = if radix == 2 {
                digits.len() as u32
            } else {
                digits.len() as u32 * 4
            };
            let bytes = literal_bytes(digits, radix, width, value)?;
            return Ok(LoweredExpr {
                node: sink_result(self.sink.bv_const(&bytes, width))?,
                sort: SmtSort::Bv(width),
            });
        }
        locals
            .get(value)
            .or_else(|| self.env.get(value))
            .copied()
            .ok_or_else(|| {
                FrontendError::invalid("SMT-LIB symbol", format!("undefined symbol {value:?}"))
            })
    }

    fn list_expr(
        &mut self,
        items: &[SExpr],
        locals: &mut HashMap<String, LoweredExpr<S::Node>>,
    ) -> Result<LoweredExpr<S::Node>> {
        if items.is_empty() {
            return Err(FrontendError::invalid("SMT-LIB expression", "empty list"));
        }
        if let SExpr::List(op_items) = &items[0] {
            return self.indexed_op(op_items, &items[1..], locals);
        }
        if atom(&items[0])? == "_" {
            return self.indexed_literal(items);
        }
        let op = atom(&items[0])?;
        if let Some(function) = self.functions.get(op).cloned() {
            return self.function_call(op, &function, &items[1..], locals);
        }
        match op {
            "!" => {
                let Some((term, _)) = parse_annotation(items)? else {
                    return Err(FrontendError::invalid("annotation", "expected annotation"));
                };
                self.expr_with_locals(term, locals)
            }
            "let" => self.let_expr(items, locals),
            "ite" => self.ite_expr(items, locals),
            "=" => self.equals(&items[1..], locals),
            "distinct" => self.distinct(&items[1..], locals),
            "not" => self.unary_bool(items, locals, S::bool_not),
            "and" => self.fold_bool(&items[1..], locals, true, S::bool_and),
            "or" => self.fold_bool(&items[1..], locals, false, S::bool_or),
            "=>" => self.binary_bool(items, locals, S::bool_implies),
            "xor" => self.binary_bool(items, locals, S::bool_xor),
            "bvnot" => self.unary_bv(items, locals, S::bv_not),
            "bvneg" => self.unary_bv(items, locals, S::bv_neg),
            "bvand" => self.fold_bv(items, locals, S::bv_and),
            "bvnand" => self.inverted_fold_bv(items, locals, S::bv_and),
            "bvor" => self.fold_bv(items, locals, S::bv_or),
            "bvnor" => self.inverted_fold_bv(items, locals, S::bv_or),
            "bvxor" => self.fold_bv(items, locals, S::bv_xor),
            "bvxnor" => self.inverted_fold_bv(items, locals, S::bv_xor),
            "bvadd" => self.fold_bv(items, locals, S::bv_add),
            "bvsub" => self.binary_bv(items, locals, S::bv_sub),
            "bvmul" => self.fold_bv(items, locals, S::bv_mul),
            "bvudiv" => self.binary_bv(items, locals, S::bv_udiv),
            "bvurem" => self.binary_bv(items, locals, S::bv_urem),
            "bvsdiv" => self.binary_bv(items, locals, S::bv_sdiv),
            "bvsrem" => self.binary_bv(items, locals, S::bv_srem),
            "bvsmod" => self.binary_bv(items, locals, S::bv_smod),
            "bvshl" => self.binary_bv(items, locals, S::bv_shl),
            "bvlshr" => self.binary_bv(items, locals, S::bv_lshr),
            "bvashr" => self.binary_bv(items, locals, S::bv_ashr),
            "bvcomp" => self.bvcomp(items, locals),
            "concat" => self.concat(items, locals),
            "bvult" => self.bv_cmp(items, locals, S::bv_ult),
            "bvule" => self.bv_cmp(items, locals, S::bv_ule),
            "bvugt" => self.bv_cmp(items, locals, |sink, a, b| sink.bv_ult(b, a)),
            "bvuge" => self.bv_cmp(items, locals, |sink, a, b| sink.bv_ule(b, a)),
            "bvslt" => self.bv_cmp(items, locals, S::bv_slt),
            "bvsle" => self.bv_cmp(items, locals, S::bv_sle),
            "bvsgt" => self.bv_cmp(items, locals, |sink, a, b| sink.bv_slt(b, a)),
            "bvsge" => self.bv_cmp(items, locals, |sink, a, b| sink.bv_sle(b, a)),
            "bvuaddo" | "uaddo" => self.overflow_cmp(items, locals, S::uadd_ovf),
            "bvsaddo" | "saddo" => self.overflow_cmp(items, locals, S::sadd_ovf),
            "bvusubo" | "usubo" => self.overflow_cmp(items, locals, S::usub_ovf),
            "bvssubo" | "ssubo" => self.overflow_cmp(items, locals, S::ssub_ovf),
            "bvumulo" | "umulo" => self.overflow_cmp(items, locals, S::umul_ovf),
            "bvsmulo" | "smulo" => self.overflow_cmp(items, locals, S::smul_ovf),
            "bvsdivo" | "sdivo" => self.overflow_cmp(items, locals, S::sdiv_ovf),
            "bvnego" | "nego" => self.unary_overflow(items, locals, S::neg_ovf),
            other => Err(FrontendError::Unsupported(format!(
                "SMT-LIB operator {other}"
            ))),
        }
    }

    fn function_call(
        &mut self,
        name: &str,
        function: &FunctionBinding,
        args: &[SExpr],
        locals: &mut HashMap<String, LoweredExpr<S::Node>>,
    ) -> Result<LoweredExpr<S::Node>> {
        if args.len() != function.params.len() {
            return Err(FrontendError::invalid(
                "define-fun application",
                format!(
                    "{name} expected {} arguments, got {}",
                    function.params.len(),
                    args.len()
                ),
            ));
        }
        if self.expansion_depth >= self.options.max_macro_expansion_depth {
            return Err(FrontendError::invalid(
                "define-fun application",
                "macro expansion depth exceeded",
            ));
        }
        let mut expansion_locals = HashMap::with_capacity(function.params.len());
        for ((param_name, param_sort), arg) in function.params.iter().zip(args) {
            let binding = self.expr_with_locals(arg, locals)?;
            expect_sort(binding.sort, *param_sort, "define-fun application")?;
            expansion_locals.insert(param_name.clone(), binding);
        }
        self.expansion_depth += 1;
        let result = self.expr_with_locals(&function.body, &mut expansion_locals);
        self.expansion_depth -= 1;
        let binding = result?;
        expect_sort(binding.sort, function.result, "define-fun result")?;
        Ok(binding)
    }

    fn indexed_literal(&mut self, items: &[SExpr]) -> Result<LoweredExpr<S::Node>> {
        let (bytes, width) = indexed_bv_literal_bytes(items)?;
        Ok(LoweredExpr {
            node: sink_result(self.sink.bv_const(&bytes, width))?,
            sort: SmtSort::Bv(width),
        })
    }

    fn indexed_op(
        &mut self,
        op_items: &[SExpr],
        args: &[SExpr],
        locals: &mut HashMap<String, LoweredExpr<S::Node>>,
    ) -> Result<LoweredExpr<S::Node>> {
        if op_items.len() == 4 && atom(&op_items[0])? == "_" && atom(&op_items[1])? == "extract" {
            if args.len() != 1 {
                return Err(FrontendError::invalid("extract", "expected one argument"));
            }
            let hi = parse_u32_atom(&op_items[2], "extract high")?;
            let lo = parse_u32_atom(&op_items[3], "extract low")?;
            let x = self.expr_with_locals(&args[0], locals)?;
            let SmtSort::Bv(_) = x.sort else {
                return Err(FrontendError::invalid("extract", "argument is not BV"));
            };
            let result_width = hi
                .checked_sub(lo)
                .and_then(|value| value.checked_add(1))
                .ok_or_else(|| FrontendError::invalid("extract", "invalid bounds"))?;
            return Ok(LoweredExpr {
                node: sink_result(self.sink.bv_extract(x.node, hi, lo))?,
                sort: SmtSort::Bv(result_width),
            });
        }
        if op_items.len() == 3 && atom(&op_items[0])? == "_" {
            if args.len() != 1 {
                return Err(FrontendError::invalid(
                    "indexed operator",
                    "expected one argument",
                ));
            }
            let amount = parse_u32_atom(&op_items[2], "indexed amount")?;
            let x = self.expr_with_locals(&args[0], locals)?;
            let SmtSort::Bv(width) = x.sort else {
                return Err(FrontendError::invalid(
                    "indexed operator",
                    "argument is not BV",
                ));
            };
            return match atom(&op_items[1])? {
                "zero_extend" => {
                    Ok(LoweredExpr {
                        node: sink_result(self.sink.bv_zext(x.node, amount))?,
                        sort: SmtSort::Bv(width.checked_add(amount).ok_or_else(|| {
                            FrontendError::invalid("zero_extend", "width overflow")
                        })?),
                    })
                }
                "sign_extend" => {
                    Ok(LoweredExpr {
                        node: sink_result(self.sink.bv_sext(x.node, amount))?,
                        sort: SmtSort::Bv(width.checked_add(amount).ok_or_else(|| {
                            FrontendError::invalid("sign_extend", "width overflow")
                        })?),
                    })
                }
                "repeat" => {
                    if amount == 0 {
                        return Err(FrontendError::invalid("repeat", "count must be positive"));
                    }
                    Ok(LoweredExpr {
                        node: sink_result(self.sink.bv_repeat(x.node, amount))?,
                        sort: SmtSort::Bv(
                            width.checked_mul(amount).ok_or_else(|| {
                                FrontendError::invalid("repeat", "width overflow")
                            })?,
                        ),
                    })
                }
                "rotate_left" => Ok(LoweredExpr {
                    node: sink_result(self.sink.bv_rotate_left(x.node, amount))?,
                    sort: SmtSort::Bv(width),
                }),
                "rotate_right" => Ok(LoweredExpr {
                    node: sink_result(self.sink.bv_rotate_right(x.node, amount))?,
                    sort: SmtSort::Bv(width),
                }),
                _ => Err(FrontendError::Unsupported("indexed operator".to_owned())),
            };
        }
        Err(FrontendError::Unsupported("indexed operator".to_owned()))
    }

    fn let_expr(
        &mut self,
        items: &[SExpr],
        locals: &mut HashMap<String, LoweredExpr<S::Node>>,
    ) -> Result<LoweredExpr<S::Node>> {
        expect_len(items, 3, "let")?;
        let bindings = match &items[1] {
            SExpr::List(items) => items,
            _ => return Err(FrontendError::invalid("let", "expected binding list")),
        };
        let mut parsed_bindings = Vec::with_capacity(bindings.len());
        for binding in bindings {
            let pair = match binding {
                SExpr::List(pair) if pair.len() == 2 => pair,
                _ => return Err(FrontendError::invalid("let", "bad binding")),
            };
            let name = atom(&pair[0])?.to_owned();
            if parsed_bindings
                .iter()
                .any(|(existing, _): &(String, LoweredExpr<S::Node>)| existing == &name)
            {
                return Err(FrontendError::invalid(
                    "let",
                    format!("duplicate binding {name:?}"),
                ));
            }
            let value = self.expr_with_locals(&pair[1], locals)?;
            parsed_bindings.push((name, value));
        }
        let mut nested = locals.clone();
        for (name, value) in parsed_bindings {
            nested.insert(name, value);
        }
        self.expr_with_locals(&items[2], &mut nested)
    }

    fn ite_expr(
        &mut self,
        items: &[SExpr],
        locals: &mut HashMap<String, LoweredExpr<S::Node>>,
    ) -> Result<LoweredExpr<S::Node>> {
        expect_len(items, 4, "ite")?;
        let c = self.expr_with_locals(&items[1], locals)?;
        let t = self.expr_with_locals(&items[2], locals)?;
        let e = self.expr_with_locals(&items[3], locals)?;
        expect_sort(c.sort, SmtSort::Bool, "ite condition")?;
        match (t.sort, e.sort) {
            (SmtSort::Bool, SmtSort::Bool) => Ok(LoweredExpr {
                node: sink_result(self.sink.bool_ite(c.node, t.node, e.node))?,
                sort: SmtSort::Bool,
            }),
            (SmtSort::Bv(w1), SmtSort::Bv(w2)) if w1 == w2 => Ok(LoweredExpr {
                node: sink_result(self.sink.bv_ite(c.node, t.node, e.node))?,
                sort: SmtSort::Bv(w1),
            }),
            _ => Err(FrontendError::invalid("ite", "branch sorts differ")),
        }
    }

    fn equals(
        &mut self,
        args: &[SExpr],
        locals: &mut HashMap<String, LoweredExpr<S::Node>>,
    ) -> Result<LoweredExpr<S::Node>> {
        if args.len() < 2 {
            return Err(FrontendError::invalid(
                "=",
                "expected at least two arguments",
            ));
        }
        let mut result = sink_result(self.sink.bool_const(true))?;
        let mut previous = self.expr_with_locals(&args[0], locals)?;
        for arg in &args[1..] {
            let next = self.expr_with_locals(arg, locals)?;
            let eq = self.equality_node(previous, next, "=")?;
            result = sink_result(self.sink.bool_and(result, eq))?;
            previous = next;
        }
        Ok(LoweredExpr {
            node: result,
            sort: SmtSort::Bool,
        })
    }

    fn distinct(
        &mut self,
        args: &[SExpr],
        locals: &mut HashMap<String, LoweredExpr<S::Node>>,
    ) -> Result<LoweredExpr<S::Node>> {
        if args.len() < 2 {
            return Err(FrontendError::invalid(
                "distinct",
                "expected at least two arguments",
            ));
        }
        let values = args
            .iter()
            .map(|arg| self.expr_with_locals(arg, locals))
            .collect::<Result<Vec<_>>>()?;
        let mut result = sink_result(self.sink.bool_const(true))?;
        for i in 0..values.len() {
            for j in (i + 1)..values.len() {
                let eq = self.equality_node(values[i], values[j], "distinct")?;
                let ne = sink_result(self.sink.bool_not(eq))?;
                result = sink_result(self.sink.bool_and(result, ne))?;
            }
        }
        Ok(LoweredExpr {
            node: result,
            sort: SmtSort::Bool,
        })
    }

    fn equality_node(
        &mut self,
        a: LoweredExpr<S::Node>,
        b: LoweredExpr<S::Node>,
        context: &'static str,
    ) -> Result<S::Node> {
        match (a.sort, b.sort) {
            (SmtSort::Bool, SmtSort::Bool) => sink_result(self.sink.bool_eq(a.node, b.node)),
            (SmtSort::Bv(w1), SmtSort::Bv(w2)) if w1 == w2 => {
                sink_result(self.sink.bv_eq(a.node, b.node))
            }
            _ => Err(FrontendError::invalid(context, "argument sorts differ")),
        }
    }

    fn unary_bool(
        &mut self,
        items: &[SExpr],
        locals: &mut HashMap<String, LoweredExpr<S::Node>>,
        f: UnarySinkOp<S>,
    ) -> Result<LoweredExpr<S::Node>> {
        expect_len(items, 2, "unary bool")?;
        let x = self.expr_with_locals(&items[1], locals)?;
        expect_sort(x.sort, SmtSort::Bool, "unary bool")?;
        Ok(LoweredExpr {
            node: sink_result(f(self.sink, x.node))?,
            sort: SmtSort::Bool,
        })
    }

    fn binary_bool(
        &mut self,
        items: &[SExpr],
        locals: &mut HashMap<String, LoweredExpr<S::Node>>,
        f: BinarySinkOp<S>,
    ) -> Result<LoweredExpr<S::Node>> {
        expect_len(items, 3, "binary bool")?;
        let a = self.expr_with_locals(&items[1], locals)?;
        let b = self.expr_with_locals(&items[2], locals)?;
        expect_sort(a.sort, SmtSort::Bool, "binary bool")?;
        expect_sort(b.sort, SmtSort::Bool, "binary bool")?;
        Ok(LoweredExpr {
            node: sink_result(f(self.sink, a.node, b.node))?,
            sort: SmtSort::Bool,
        })
    }

    fn fold_bool(
        &mut self,
        args: &[SExpr],
        locals: &mut HashMap<String, LoweredExpr<S::Node>>,
        identity: bool,
        f: BinarySinkOp<S>,
    ) -> Result<LoweredExpr<S::Node>> {
        if args.is_empty() {
            return Ok(LoweredExpr {
                node: sink_result(self.sink.bool_const(identity))?,
                sort: SmtSort::Bool,
            });
        }
        let mut cur = self.expr_with_locals(&args[0], locals)?;
        expect_sort(cur.sort, SmtSort::Bool, "fold bool")?;
        for arg in &args[1..] {
            let next = self.expr_with_locals(arg, locals)?;
            expect_sort(next.sort, SmtSort::Bool, "fold bool")?;
            cur = LoweredExpr {
                node: sink_result(f(self.sink, cur.node, next.node))?,
                sort: SmtSort::Bool,
            };
        }
        Ok(cur)
    }

    fn unary_bv(
        &mut self,
        items: &[SExpr],
        locals: &mut HashMap<String, LoweredExpr<S::Node>>,
        f: UnarySinkOp<S>,
    ) -> Result<LoweredExpr<S::Node>> {
        expect_len(items, 2, "unary BV")?;
        let x = self.expr_with_locals(&items[1], locals)?;
        let SmtSort::Bv(width) = x.sort else {
            return Err(FrontendError::invalid("unary BV", "argument is not BV"));
        };
        Ok(LoweredExpr {
            node: sink_result(f(self.sink, x.node))?,
            sort: SmtSort::Bv(width),
        })
    }

    fn binary_bv(
        &mut self,
        items: &[SExpr],
        locals: &mut HashMap<String, LoweredExpr<S::Node>>,
        f: BinarySinkOp<S>,
    ) -> Result<LoweredExpr<S::Node>> {
        expect_len(items, 3, "binary BV")?;
        let a = self.expr_with_locals(&items[1], locals)?;
        let b = self.expr_with_locals(&items[2], locals)?;
        self.binary_bv_bindings(a, b, f, "binary BV")
    }

    fn fold_bv(
        &mut self,
        items: &[SExpr],
        locals: &mut HashMap<String, LoweredExpr<S::Node>>,
        f: BinarySinkOp<S>,
    ) -> Result<LoweredExpr<S::Node>> {
        if items.len() < 2 {
            return Err(FrontendError::invalid(
                "BV fold",
                "expected at least one argument",
            ));
        }
        let mut cur = self.expr_with_locals(&items[1], locals)?;
        let SmtSort::Bv(_) = cur.sort else {
            return Err(FrontendError::invalid("BV fold", "argument is not BV"));
        };
        for item in &items[2..] {
            let next = self.expr_with_locals(item, locals)?;
            cur = self.binary_bv_bindings(cur, next, f, "BV fold")?;
        }
        Ok(cur)
    }

    fn inverted_fold_bv(
        &mut self,
        items: &[SExpr],
        locals: &mut HashMap<String, LoweredExpr<S::Node>>,
        f: BinarySinkOp<S>,
    ) -> Result<LoweredExpr<S::Node>> {
        let value = self.fold_bv(items, locals, f)?;
        Ok(LoweredExpr {
            node: sink_result(self.sink.bv_not(value.node))?,
            sort: value.sort,
        })
    }

    fn binary_bv_bindings(
        &mut self,
        a: LoweredExpr<S::Node>,
        b: LoweredExpr<S::Node>,
        f: BinarySinkOp<S>,
        context: &'static str,
    ) -> Result<LoweredExpr<S::Node>> {
        let (SmtSort::Bv(w1), SmtSort::Bv(w2)) = (a.sort, b.sort) else {
            return Err(FrontendError::invalid(context, "argument is not BV"));
        };
        if w1 != w2 {
            return Err(FrontendError::invalid(context, "width mismatch"));
        }
        Ok(LoweredExpr {
            node: sink_result(f(self.sink, a.node, b.node))?,
            sort: SmtSort::Bv(w1),
        })
    }

    fn bvcomp(
        &mut self,
        items: &[SExpr],
        locals: &mut HashMap<String, LoweredExpr<S::Node>>,
    ) -> Result<LoweredExpr<S::Node>> {
        expect_len(items, 3, "bvcomp")?;
        let a = self.expr_with_locals(&items[1], locals)?;
        let b = self.expr_with_locals(&items[2], locals)?;
        let (SmtSort::Bv(w1), SmtSort::Bv(w2)) = (a.sort, b.sort) else {
            return Err(FrontendError::invalid("bvcomp", "argument is not BV"));
        };
        if w1 != w2 {
            return Err(FrontendError::invalid("bvcomp", "width mismatch"));
        }
        let eq = sink_result(self.sink.bv_eq(a.node, b.node))?;
        let one = sink_result(self.sink.bv_const(&[1], 1))?;
        let zero = sink_result(self.sink.bv_const(&[0], 1))?;
        Ok(LoweredExpr {
            node: sink_result(self.sink.bv_ite(eq, one, zero))?,
            sort: SmtSort::Bv(1),
        })
    }

    fn concat(
        &mut self,
        items: &[SExpr],
        locals: &mut HashMap<String, LoweredExpr<S::Node>>,
    ) -> Result<LoweredExpr<S::Node>> {
        expect_len(items, 3, "concat")?;
        let a = self.expr_with_locals(&items[1], locals)?;
        let b = self.expr_with_locals(&items[2], locals)?;
        let (SmtSort::Bv(w1), SmtSort::Bv(w2)) = (a.sort, b.sort) else {
            return Err(FrontendError::invalid("concat", "argument is not BV"));
        };
        Ok(LoweredExpr {
            node: sink_result(self.sink.bv_concat(a.node, b.node))?,
            sort: SmtSort::Bv(
                w1.checked_add(w2)
                    .ok_or_else(|| FrontendError::invalid("concat", "width overflow"))?,
            ),
        })
    }

    fn bv_cmp(
        &mut self,
        items: &[SExpr],
        locals: &mut HashMap<String, LoweredExpr<S::Node>>,
        f: BinarySinkOp<S>,
    ) -> Result<LoweredExpr<S::Node>> {
        let value = self.binary_bv(items, locals, f)?;
        Ok(LoweredExpr {
            node: value.node,
            sort: SmtSort::Bool,
        })
    }

    fn overflow_cmp(
        &mut self,
        items: &[SExpr],
        locals: &mut HashMap<String, LoweredExpr<S::Node>>,
        f: BinarySinkOp<S>,
    ) -> Result<LoweredExpr<S::Node>> {
        self.bv_cmp(items, locals, f)
    }

    fn unary_overflow(
        &mut self,
        items: &[SExpr],
        locals: &mut HashMap<String, LoweredExpr<S::Node>>,
        f: UnarySinkOp<S>,
    ) -> Result<LoweredExpr<S::Node>> {
        let value = self.unary_bv(items, locals, f)?;
        Ok(LoweredExpr {
            node: value.node,
            sort: SmtSort::Bool,
        })
    }
}

struct SExprAction;

fn sexpr_atom(value: impl Into<String>) -> SExpr {
    SExpr::Atom(value.into())
}

fn sexpr_list(items: impl IntoIterator<Item = SExpr>) -> SExpr {
    SExpr::List(items.into_iter().collect())
}

fn indexed_symbol(symbol: String, indices: Vec<SExpr>) -> SExpr {
    if indices.is_empty() {
        sexpr_atom(symbol)
    } else {
        let mut items = Vec::with_capacity(indices.len() + 2);
        items.push(sexpr_atom("_"));
        items.push(sexpr_atom(symbol));
        items.extend(indices);
        sexpr_list(items)
    }
}

fn byte_bits(bytes: &[u8], len: usize) -> String {
    let mut out = String::with_capacity(len + 2);
    out.push_str("#b");
    for bit in (0..len).rev() {
        let byte = bytes[bit / 8];
        out.push(if ((byte >> (bit % 8)) & 1) != 0 {
            '1'
        } else {
            '0'
        });
    }
    out
}

fn byte_hex(bytes: &[u8], len: usize) -> String {
    let mut out = String::with_capacity(len + 2);
    out.push_str("#x");
    for nibble in (0..len).rev() {
        let byte = bytes[nibble / 2];
        let value = (byte >> ((nibble % 2) * 4)) & 0xf;
        out.push(char::from_digit(u32::from(value), 16).expect("hex digit"));
    }
    out
}

fn command(name: &str, args: impl IntoIterator<Item = SExpr>) -> SExpr {
    let mut items = vec![sexpr_atom(name)];
    items.extend(args);
    sexpr_list(items)
}

fn vars_to_sexpr(vars: Vec<(String, SExpr)>) -> SExpr {
    sexpr_list(
        vars.into_iter()
            .map(|(name, sort)| sexpr_list([sexpr_atom(name), sort])),
    )
}

impl ActionOnString for SExprAction {
    type Str = String;

    fn on_string(&mut self, _range: Range, s: String) -> ParsingResult<Self::Str> {
        Ok(s)
    }
}

impl ActionOnConstant for SExprAction {
    type Constant = SExpr;

    fn on_constant_binary(
        &mut self,
        _range: Range,
        bytes: Vec<u8>,
        len: usize,
    ) -> ParsingResult<Self::Constant> {
        Ok(sexpr_atom(byte_bits(&bytes, len)))
    }

    fn on_constant_hexadecimal(
        &mut self,
        _range: Range,
        bytes: Vec<u8>,
        len: usize,
    ) -> ParsingResult<Self::Constant> {
        Ok(sexpr_atom(byte_hex(&bytes, len)))
    }

    fn on_constant_decimal(
        &mut self,
        _range: Range,
        decimal: DBig,
    ) -> ParsingResult<Self::Constant> {
        Ok(sexpr_atom(decimal.to_string()))
    }

    fn on_constant_numeral(
        &mut self,
        _range: Range,
        numeral: UBig,
    ) -> ParsingResult<Self::Constant> {
        Ok(sexpr_atom(numeral.to_string()))
    }

    fn on_constant_string(
        &mut self,
        _range: Range,
        string: Self::Str,
    ) -> ParsingResult<Self::Constant> {
        Ok(sexpr_atom(string))
    }

    fn on_constant_bool(&mut self, _range: Range, boolean: bool) -> ParsingResult<Self::Constant> {
        Ok(sexpr_atom(if boolean { "true" } else { "false" }))
    }
}

impl ActionOnIndex for SExprAction {
    type Index = SExpr;

    fn on_index_numeral(&mut self, _range: Range, index: UBig) -> ParsingResult<Self::Index> {
        Ok(sexpr_atom(index.to_string()))
    }

    fn on_index_symbol(&mut self, _range: Range, index: Self::Str) -> ParsingResult<Self::Index> {
        Ok(sexpr_atom(index))
    }

    fn on_index_hexadecimal(
        &mut self,
        _range: Range,
        bytes: Vec<u8>,
        len: usize,
    ) -> ParsingResult<Self::Index> {
        Ok(sexpr_atom(byte_hex(&bytes, len)))
    }
}

impl ActionOnIdentifier for SExprAction {
    type Identifier = SExpr;

    fn on_identifier(
        &mut self,
        _range: Range,
        symbol: Self::Str,
        indices: Vec<Self::Index>,
    ) -> ParsingResult<Self::Identifier> {
        Ok(indexed_symbol(symbol, indices))
    }
}

impl ActionOnAttribute for SExprAction {
    type Term = SExpr;
    type Attribute = SExpr;

    fn on_attribute_keyword(
        &mut self,
        _range: Range,
        keyword: Keyword,
    ) -> ParsingResult<Self::Attribute> {
        Ok(sexpr_list([sexpr_atom(keyword.to_string())]))
    }

    fn on_attribute_constant(
        &mut self,
        _range: Range,
        keyword: Keyword,
        constant: Self::Constant,
    ) -> ParsingResult<Self::Attribute> {
        Ok(sexpr_list([sexpr_atom(keyword.to_string()), constant]))
    }

    fn on_attribute_symbol(
        &mut self,
        _range: Range,
        keyword: Keyword,
        symbol: Self::Str,
    ) -> ParsingResult<Self::Attribute> {
        Ok(sexpr_list([
            sexpr_atom(keyword.to_string()),
            sexpr_atom(symbol),
        ]))
    }

    fn on_attribute_named(
        &mut self,
        _range: Range,
        name: Self::Str,
    ) -> ParsingResult<Self::Attribute> {
        Ok(sexpr_list([sexpr_atom(":named"), sexpr_atom(name)]))
    }

    fn on_attribute_pattern(
        &mut self,
        _range: Range,
        patterns: Vec<Self::Term>,
    ) -> ParsingResult<Self::Attribute> {
        Ok(command(":pattern", patterns))
    }
}

impl ActionOnSort for SExprAction {
    type Sort = SExpr;

    fn on_sort(
        &mut self,
        _range: Range,
        identifier: Self::Identifier,
        args: Vec<Self::Sort>,
    ) -> ParsingResult<Self::Sort> {
        if args.is_empty() {
            Ok(identifier)
        } else {
            let mut items = vec![identifier];
            items.extend(args);
            Ok(sexpr_list(items))
        }
    }
}

impl ActionOnTerm for SExprAction {
    fn on_term_constant(
        &mut self,
        _range: Range,
        constant: Self::Constant,
    ) -> ParsingResult<Self::Term> {
        Ok(constant)
    }

    fn on_term_identifier(
        &mut self,
        _range: Range,
        identifier: Self::Identifier,
        sort: Option<Self::Sort>,
    ) -> ParsingResult<Self::Term> {
        Ok(match sort {
            Some(sort) => command("as", [identifier, sort]),
            None => identifier,
        })
    }

    fn on_term_app(
        &mut self,
        _range: Range,
        identifier: Self::Identifier,
        sort: Option<Self::Sort>,
        args: Vec<Self::Term>,
    ) -> ParsingResult<Self::Term> {
        let head = match sort {
            Some(sort) => command("as", [identifier, sort]),
            None => identifier,
        };
        let mut items = vec![head];
        items.extend(args);
        Ok(sexpr_list(items))
    }

    fn on_term_let(
        &mut self,
        _range: Range,
        bindings: Vec<(Self::Str, Self::Term)>,
        body: Self::Term,
    ) -> ParsingResult<Self::Term> {
        let binding_list = sexpr_list(
            bindings
                .into_iter()
                .map(|(name, term)| sexpr_list([sexpr_atom(name), term])),
        );
        Ok(command("let", [binding_list, body]))
    }

    fn on_term_lambda(
        &mut self,
        _range: Range,
        names: Vec<(Self::Str, Self::Sort)>,
        body: Self::Term,
    ) -> ParsingResult<Self::Term> {
        Ok(command("lambda", [vars_to_sexpr(names), body]))
    }

    fn on_term_exists(
        &mut self,
        _range: Range,
        names: Vec<(Self::Str, Self::Sort)>,
        body: Self::Term,
    ) -> ParsingResult<Self::Term> {
        Ok(command("exists", [vars_to_sexpr(names), body]))
    }

    fn on_term_forall(
        &mut self,
        _range: Range,
        names: Vec<(Self::Str, Self::Sort)>,
        body: Self::Term,
    ) -> ParsingResult<Self::Term> {
        Ok(command("forall", [vars_to_sexpr(names), body]))
    }

    fn on_term_match(
        &mut self,
        _range: Range,
        scrutinee: Self::Term,
        cases: Vec<(Pattern<Self::Str>, Self::Term)>,
    ) -> ParsingResult<Self::Term> {
        let case_exprs = cases.into_iter().map(|(_, body)| body);
        Ok(command("match", [scrutinee, sexpr_list(case_exprs)]))
    }

    fn on_term_annotated(
        &mut self,
        _range: Range,
        t: Self::Term,
        attributes: Vec<Self::Attribute>,
    ) -> ParsingResult<Self::Term> {
        let mut items = vec![sexpr_atom("!"), t];
        for attribute in attributes {
            match attribute {
                SExpr::List(values) => items.extend(values),
                atom @ SExpr::Atom(_) => items.push(atom),
            }
        }
        Ok(sexpr_list(items))
    }
}

impl ParsingAction for SExprAction {
    type Command = SExpr;

    fn on_command_assert(&mut self, _range: Range, t: Self::Term) -> ParsingResult<Self::Command> {
        Ok(command("assert", [t]))
    }

    fn on_command_check_sat(&mut self, _range: Range) -> ParsingResult<Self::Command> {
        Ok(command("check-sat", []))
    }

    fn on_command_check_sat_assuming(
        &mut self,
        _range: Range,
        terms: Vec<Self::Term>,
    ) -> ParsingResult<Self::Command> {
        Ok(command("check-sat-assuming", [sexpr_list(terms)]))
    }

    fn on_command_declare_const(
        &mut self,
        _range: Range,
        name: Self::Str,
        sort: Self::Sort,
    ) -> ParsingResult<Self::Command> {
        Ok(command("declare-const", [sexpr_atom(name), sort]))
    }

    fn on_command_declare_datatype(
        &mut self,
        _range: Range,
        name: Self::Str,
        _datatype: DatatypeDec<Self::Str, Self::Sort>,
    ) -> ParsingResult<Self::Command> {
        Ok(command("declare-datatype", [sexpr_atom(name)]))
    }

    fn on_command_declare_datatypes(
        &mut self,
        _range: Range,
        _defs: Vec<DatatypeDef<Self::Str, Self::Sort>>,
    ) -> ParsingResult<Self::Command> {
        Ok(command("declare-datatypes", []))
    }

    fn on_command_declare_fun(
        &mut self,
        _range: Range,
        name: Self::Str,
        input_sorts: Vec<Self::Sort>,
        out_sort: Self::Sort,
    ) -> ParsingResult<Self::Command> {
        Ok(command(
            "declare-fun",
            [sexpr_atom(name), sexpr_list(input_sorts), out_sort],
        ))
    }

    fn on_command_declare_sort(
        &mut self,
        _range: Range,
        name: Self::Str,
        arity: UBig,
    ) -> ParsingResult<Self::Command> {
        Ok(command(
            "declare-sort",
            [sexpr_atom(name), sexpr_atom(arity.to_string())],
        ))
    }

    fn on_command_declare_sort_parameter(
        &mut self,
        _range: Range,
        name: Self::Str,
    ) -> ParsingResult<Self::Command> {
        Ok(command("declare-sort-parameter", [sexpr_atom(name)]))
    }

    fn on_command_define_const(
        &mut self,
        _range: Range,
        name: Self::Str,
        sort: Self::Sort,
        term: Self::Term,
    ) -> ParsingResult<Self::Command> {
        Ok(command("define-const", [sexpr_atom(name), sort, term]))
    }

    fn on_command_define_fun(
        &mut self,
        _range: Range,
        definition: FunctionDef<Self::Str, Self::Sort, Self::Term>,
    ) -> ParsingResult<Self::Command> {
        Ok(command(
            "define-fun",
            [
                sexpr_atom(definition.name),
                vars_to_sexpr(definition.vars),
                definition.out_sort,
                definition.body,
            ],
        ))
    }

    fn on_command_define_fun_rec(
        &mut self,
        _range: Range,
        definition: FunctionDef<Self::Str, Self::Sort, Self::Term>,
    ) -> ParsingResult<Self::Command> {
        Ok(command(
            "define-fun-rec",
            [
                sexpr_atom(definition.name),
                vars_to_sexpr(definition.vars),
                definition.out_sort,
                definition.body,
            ],
        ))
    }

    fn on_command_define_funs_rec(
        &mut self,
        _range: Range,
        _definitions: Vec<FunctionDef<Self::Str, Self::Sort, Self::Term>>,
    ) -> ParsingResult<Self::Command> {
        Ok(command("define-funs-rec", []))
    }

    fn on_command_define_sort(
        &mut self,
        _range: Range,
        name: Self::Str,
        params: Vec<Self::Str>,
        sort: Self::Sort,
    ) -> ParsingResult<Self::Command> {
        Ok(command(
            "define-sort",
            [
                sexpr_atom(name),
                sexpr_list(params.into_iter().map(sexpr_atom)),
                sort,
            ],
        ))
    }

    fn on_command_echo(&mut self, _range: Range, s: Self::Str) -> ParsingResult<Self::Command> {
        Ok(command("echo", [sexpr_atom(s)]))
    }

    fn on_command_exit(&mut self, _range: Range) -> ParsingResult<Self::Command> {
        Ok(command("exit", []))
    }

    fn on_command_get_assertions(&mut self, _range: Range) -> ParsingResult<Self::Command> {
        Ok(command("get-assertions", []))
    }

    fn on_command_get_assignment(&mut self, _range: Range) -> ParsingResult<Self::Command> {
        Ok(command("get-assignment", []))
    }

    fn on_command_get_info(&mut self, _range: Range, kw: Keyword) -> ParsingResult<Self::Command> {
        Ok(command("get-info", [sexpr_atom(kw.to_string())]))
    }

    fn on_command_get_model(&mut self, _range: Range) -> ParsingResult<Self::Command> {
        Ok(command("get-model", []))
    }

    fn on_command_get_option(
        &mut self,
        _range: Range,
        kw: Keyword,
    ) -> ParsingResult<Self::Command> {
        Ok(command("get-option", [sexpr_atom(kw.to_string())]))
    }

    fn on_command_get_proof(&mut self, _range: Range) -> ParsingResult<Self::Command> {
        Ok(command("get-proof", []))
    }

    fn on_command_get_unsat_assumptions(&mut self, _range: Range) -> ParsingResult<Self::Command> {
        Ok(command("get-unsat-assumptions", []))
    }

    fn on_command_get_unsat_core(&mut self, _range: Range) -> ParsingResult<Self::Command> {
        Ok(command("get-unsat-core", []))
    }

    fn on_command_get_value(
        &mut self,
        _range: Range,
        ts: Vec<Self::Term>,
    ) -> ParsingResult<Self::Command> {
        Ok(command("get-value", [sexpr_list(ts)]))
    }

    fn on_command_pop(&mut self, _range: Range, lvl: UBig) -> ParsingResult<Self::Command> {
        Ok(command("pop", [sexpr_atom(lvl.to_string())]))
    }

    fn on_command_push(&mut self, _range: Range, lvl: UBig) -> ParsingResult<Self::Command> {
        Ok(command("push", [sexpr_atom(lvl.to_string())]))
    }

    fn on_command_reset(&mut self, _range: Range) -> ParsingResult<Self::Command> {
        Ok(command("reset", []))
    }

    fn on_command_reset_assertions(&mut self, _range: Range) -> ParsingResult<Self::Command> {
        Ok(command("reset-assertions", []))
    }

    fn on_command_set_info(
        &mut self,
        _range: Range,
        attributes: Self::Attribute,
    ) -> ParsingResult<Self::Command> {
        Ok(command("set-info", [attributes]))
    }

    fn on_command_set_logic(
        &mut self,
        _range: Range,
        logic: Self::Str,
    ) -> ParsingResult<Self::Command> {
        Ok(command("set-logic", [sexpr_atom(logic)]))
    }

    fn on_command_set_option(
        &mut self,
        _range: Range,
        attribute: Self::Attribute,
    ) -> ParsingResult<Self::Command> {
        Ok(command("set-option", [attribute]))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parser_normalizes_indexed_literals_and_annotations() {
        let exprs = parse_sexprs(
            r#"
            (set-logic QF_BV)
            (declare-const |x y| (_ BitVec 4))
            (assert (! (= |x y| #xf) :named a0 :foo bar))
            (check-sat)
        "#,
        )
        .unwrap();
        assert!(
            matches!(&exprs[0], SExpr::List(items) if matches!(&items[0], SExpr::Atom(s) if s == "set-logic"))
        );
        assert!(format!("{exprs:?}").contains(":named"));
        assert!(format!("{exprs:?}").contains("#xf"));
    }

    #[test]
    fn parser_handles_block_comments() {
        let exprs = parse_sexprs("#| comment |# (check-sat)").unwrap();
        assert_eq!(
            exprs,
            vec![SExpr::List(vec![SExpr::Atom("check-sat".to_owned())])]
        );
    }
}
