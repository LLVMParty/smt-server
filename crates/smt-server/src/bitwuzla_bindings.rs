//! Hand-written FFI bindings for the official Bitwuzla C API (verified against
//! the 0.9.1 release; `build.rs` fetches it). Checked in rather than bindgen-
//! generated, so the build needs no libclang and the surface is small and
//! version-pinned. The option enum is position-sensitive across releases
//! (`PRODUCE_UNSAT_ASSUMPTIONS` moved 2 -> 3 in 0.9); the `bitwuzla_backend`
//! test is the drift canary when re-pinning.
//!
//! Only symbols used by [`crate::bitwuzla_backend`] are declared. `BitwuzlaTerm`
//! and `BitwuzlaSort` are opaque `u64` handles owned by a `BitwuzlaTermManager`,
//! valid for as long as the manager is alive.

#![allow(
    non_camel_case_types,
    non_snake_case,
    non_upper_case_globals,
    dead_code
)]

use std::os::raw::c_char;
use std::os::raw::c_void;

pub type BitwuzlaTermManager = std::ffi::c_void;
pub type BitwuzlaOptions = std::ffi::c_void;
pub type Bitwuzla = std::ffi::c_void;

pub type BitwuzlaTerm = u64;
pub type BitwuzlaSort = u64;
pub type BitwuzlaResult = std::os::raw::c_uint;
pub type BitwuzlaKind = std::os::raw::c_uint;
pub type BitwuzlaOption = std::os::raw::c_uint;

// --- check-sat results ---------------------------------------------------

pub const BITWUZLA_SAT: BitwuzlaResult = 10;
pub const BITWUZLA_UNSAT: BitwuzlaResult = 20;
pub const BITWUZLA_UNKNOWN: BitwuzlaResult = 0;

// --- options -------------------------------------------------------------

pub const BITWUZLA_OPT_PRODUCE_MODELS: BitwuzlaOption = 1;
pub const BITWUZLA_OPT_PRODUCE_UNSAT_ASSUMPTIONS: BitwuzlaOption = 3;

// --- term kinds ----------------------------------------------------------
// Boolean connectives.
pub const BITWUZLA_KIND_AND: BitwuzlaKind = 4;
pub const BITWUZLA_KIND_EQUAL: BitwuzlaKind = 6;
pub const BITWUZLA_KIND_IMPLIES: BitwuzlaKind = 8;
pub const BITWUZLA_KIND_NOT: BitwuzlaKind = 9;
pub const BITWUZLA_KIND_OR: BitwuzlaKind = 10;
pub const BITWUZLA_KIND_XOR: BitwuzlaKind = 11;
pub const BITWUZLA_KIND_ITE: BitwuzlaKind = 12;
// Bit-vector operators.
pub const BITWUZLA_KIND_BV_ADD: BitwuzlaKind = 19;
pub const BITWUZLA_KIND_BV_AND: BitwuzlaKind = 20;
pub const BITWUZLA_KIND_BV_ASHR: BitwuzlaKind = 21;
pub const BITWUZLA_KIND_BV_CONCAT: BitwuzlaKind = 23;
pub const BITWUZLA_KIND_BV_MUL: BitwuzlaKind = 26;
pub const BITWUZLA_KIND_BV_NEG: BitwuzlaKind = 28;
pub const BITWUZLA_KIND_BV_NEG_OVERFLOW: BitwuzlaKind = 29;
pub const BITWUZLA_KIND_BV_NOT: BitwuzlaKind = 31;
pub const BITWUZLA_KIND_BV_OR: BitwuzlaKind = 32;
pub const BITWUZLA_KIND_BV_SADD_OVERFLOW: BitwuzlaKind = 38;
pub const BITWUZLA_KIND_BV_SDIV_OVERFLOW: BitwuzlaKind = 39;
pub const BITWUZLA_KIND_BV_SDIV: BitwuzlaKind = 40;
pub const BITWUZLA_KIND_BV_SHL: BitwuzlaKind = 43;
pub const BITWUZLA_KIND_BV_SHR: BitwuzlaKind = 44;
pub const BITWUZLA_KIND_BV_SLE: BitwuzlaKind = 45;
pub const BITWUZLA_KIND_BV_SLT: BitwuzlaKind = 46;
pub const BITWUZLA_KIND_BV_SMOD: BitwuzlaKind = 47;
pub const BITWUZLA_KIND_BV_SMUL_OVERFLOW: BitwuzlaKind = 48;
pub const BITWUZLA_KIND_BV_SREM: BitwuzlaKind = 49;
pub const BITWUZLA_KIND_BV_SSUB_OVERFLOW: BitwuzlaKind = 50;
pub const BITWUZLA_KIND_BV_SUB: BitwuzlaKind = 51;
pub const BITWUZLA_KIND_BV_UADD_OVERFLOW: BitwuzlaKind = 52;
pub const BITWUZLA_KIND_BV_UDIV: BitwuzlaKind = 53;
pub const BITWUZLA_KIND_BV_ULE: BitwuzlaKind = 56;
pub const BITWUZLA_KIND_BV_ULT: BitwuzlaKind = 57;
pub const BITWUZLA_KIND_BV_UMUL_OVERFLOW: BitwuzlaKind = 58;
pub const BITWUZLA_KIND_BV_UREM: BitwuzlaKind = 59;
pub const BITWUZLA_KIND_BV_USUB_OVERFLOW: BitwuzlaKind = 60;
pub const BITWUZLA_KIND_BV_XOR: BitwuzlaKind = 62;
pub const BITWUZLA_KIND_BV_EXTRACT: BitwuzlaKind = 63;
pub const BITWUZLA_KIND_BV_SIGN_EXTEND: BitwuzlaKind = 67;
pub const BITWUZLA_KIND_BV_ZERO_EXTEND: BitwuzlaKind = 68;

unsafe extern "C" {
    pub fn bitwuzla_term_manager_new() -> *mut BitwuzlaTermManager;
    pub fn bitwuzla_term_manager_delete(tm: *mut BitwuzlaTermManager);

    pub fn bitwuzla_options_new() -> *mut BitwuzlaOptions;
    pub fn bitwuzla_options_delete(options: *mut BitwuzlaOptions);
    pub fn bitwuzla_set_option(options: *mut BitwuzlaOptions, option: BitwuzlaOption, val: u64);

    pub fn bitwuzla_new(
        tm: *mut BitwuzlaTermManager,
        options: *const BitwuzlaOptions,
    ) -> *mut Bitwuzla;
    pub fn bitwuzla_delete(bitwuzla: *mut Bitwuzla);

    pub fn bitwuzla_set_termination_callback(
        bitwuzla: *mut Bitwuzla,
        fun: Option<unsafe extern "C" fn(arg1: *mut c_void) -> i32>,
        state: *mut c_void,
    );

    pub fn bitwuzla_assert(bitwuzla: *mut Bitwuzla, term: BitwuzlaTerm);
    pub fn bitwuzla_check_sat(bitwuzla: *mut Bitwuzla) -> BitwuzlaResult;
    pub fn bitwuzla_check_sat_assuming(
        bitwuzla: *mut Bitwuzla,
        argc: u32,
        args: *mut BitwuzlaTerm,
    ) -> BitwuzlaResult;
    pub fn bitwuzla_get_value(bitwuzla: *mut Bitwuzla, term: BitwuzlaTerm) -> BitwuzlaTerm;
    pub fn bitwuzla_get_unsat_assumptions(
        bitwuzla: *mut Bitwuzla,
        size: *mut usize,
    ) -> *const BitwuzlaTerm;

    pub fn bitwuzla_mk_bool_sort(tm: *mut BitwuzlaTermManager) -> BitwuzlaSort;
    pub fn bitwuzla_mk_bv_sort(tm: *mut BitwuzlaTermManager, size: u64) -> BitwuzlaSort;
    pub fn bitwuzla_mk_true(tm: *mut BitwuzlaTermManager) -> BitwuzlaTerm;
    pub fn bitwuzla_mk_false(tm: *mut BitwuzlaTermManager) -> BitwuzlaTerm;
    pub fn bitwuzla_mk_const(
        tm: *mut BitwuzlaTermManager,
        sort: BitwuzlaSort,
        symbol: *const c_char,
    ) -> BitwuzlaTerm;
    pub fn bitwuzla_mk_bv_value(
        tm: *mut BitwuzlaTermManager,
        sort: BitwuzlaSort,
        value: *const c_char,
        base: u8,
    ) -> BitwuzlaTerm;
    pub fn bitwuzla_mk_bv_value_uint64(
        tm: *mut BitwuzlaTermManager,
        sort: BitwuzlaSort,
        value: u64,
    ) -> BitwuzlaTerm;

    pub fn bitwuzla_mk_term1(
        tm: *mut BitwuzlaTermManager,
        kind: BitwuzlaKind,
        arg: BitwuzlaTerm,
    ) -> BitwuzlaTerm;
    pub fn bitwuzla_mk_term2(
        tm: *mut BitwuzlaTermManager,
        kind: BitwuzlaKind,
        arg0: BitwuzlaTerm,
        arg1: BitwuzlaTerm,
    ) -> BitwuzlaTerm;
    pub fn bitwuzla_mk_term3(
        tm: *mut BitwuzlaTermManager,
        kind: BitwuzlaKind,
        arg0: BitwuzlaTerm,
        arg1: BitwuzlaTerm,
        arg2: BitwuzlaTerm,
    ) -> BitwuzlaTerm;
    pub fn bitwuzla_mk_term1_indexed1(
        tm: *mut BitwuzlaTermManager,
        kind: BitwuzlaKind,
        arg: BitwuzlaTerm,
        idx: u64,
    ) -> BitwuzlaTerm;
    pub fn bitwuzla_mk_term1_indexed2(
        tm: *mut BitwuzlaTermManager,
        kind: BitwuzlaKind,
        arg: BitwuzlaTerm,
        idx0: u64,
        idx1: u64,
    ) -> BitwuzlaTerm;

    pub fn bitwuzla_term_value_get_bool(term: BitwuzlaTerm) -> bool;
    pub fn bitwuzla_term_value_get_str_fmt(term: BitwuzlaTerm, base: u8) -> *const c_char;
}
