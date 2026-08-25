use std::path::PathBuf;
use std::sync::Arc;

use smt_server::{
    default_legacy_recording_tree, migrate_recording_tree, recording_db_path, serve_tcp,
    BinbitBackend, CobraBackend, CommandRouterBackend, QfbvsmtrsBackend, RacingBackend,
    RumbaBackend, ServerConfig, SimplifyChainBackend, Z3Backend,
};

fn main() -> std::io::Result<()> {
    let mut args = std::env::args().skip(1);
    let arg = args.next();
    if arg.as_deref() == Some("migrate-recordings") {
        migrate_recordings_command(args)?;
        return Ok(());
    }
    let addr = arg.unwrap_or_else(|| "127.0.0.1:9123".to_owned());
    let solver = Arc::new(
        RacingBackend::new(vec![
            Arc::new(Z3Backend),
            Arc::new(BinbitBackend),
            Arc::new(QfbvsmtrsBackend),
        ])
        .with_default_budget_ms(30_000),
    );
    let simplifier = Arc::new(SimplifyChainBackend::new(vec![
        Arc::new(RumbaBackend),
        Arc::new(CobraBackend::default()),
    ]));
    let backend = Arc::new(CommandRouterBackend::new(simplifier, solver));
    eprintln!(
        "smt-server listening on {addr} with rumba + cobra simplifier chain + racing solver (z3 crate + binbit + qfbvsmtrs)"
    );
    serve_tcp(addr, ServerConfig::new(backend))
}

fn migrate_recordings_command(mut args: impl Iterator<Item = String>) -> std::io::Result<()> {
    let source = args
        .next()
        .map(PathBuf::from)
        .or_else(default_legacy_recording_tree);
    let db = args.next().map(PathBuf::from).or_else(recording_db_path);
    if args.next().is_some() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "usage: smt-server migrate-recordings [SOURCE_DIR] [DB_PATH]",
        ));
    }

    let source = source.ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "could not determine default legacy recording directory; pass SOURCE_DIR explicitly",
        )
    })?;
    let db = db.ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "recording database disabled; pass DB_PATH explicitly",
        )
    })?;
    let report = migrate_recording_tree(&source, &db)?;

    eprintln!(
        "smt-server: migrated {} recording(s), {} duplicate(s), {} skipped ({} scanned)",
        report.inserted,
        report.duplicates,
        report.skipped(),
        report.scanned
    );
    Ok(())
}
