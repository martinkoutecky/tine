"""Create a disposable local dependency overlay; never modify the Cargo cache."""
import json
import pathlib
import shutil
import subprocess

metadata = json.loads(subprocess.check_output(['cargo', 'metadata', '--locked', '--format-version=1']))
package = next(p for p in metadata['packages'] if p['name'] == 'tine-storage')
source = pathlib.Path(package['manifest_path']).parent
target = pathlib.Path('vendor/tine-storage-543')
shutil.copytree(source, target, ignore=shutil.ignore_patterns('.git', 'target'))

def patch(file, old, new):
    text = file.read_text(encoding='utf-8')
    assert text.count(old) == 1, (file, old[:80], text.count(old))
    file.write_text(text.replace(old, new), encoding='utf-8')

def timer(label):
    return f'\n        if std::env::var_os("TINE_DIAGNOSE_543_VFS").is_some() {{ crate::sqlite_graph_projection::sql543_vfs_dump(); }}\n        eprintln!("SQL543 {label}={{:?}}", sql543.elapsed());\n        let sql543 = std::time::Instant::now();\n'

file = target / 'src/sqlite_graph_projection.rs'
patch(file, '    pub fn open_writable(path: &Path) -> Result<Self, MaterializationError> {',
    '    pub fn open_writable(path: &Path) -> Result<Self, MaterializationError> {\n'
    '        if std::env::var_os("TINE_DIAGNOSE_543_VFS").is_some() { sql543_vfs_install(); }')
patch(file, '            .execute_batch("PRAGMA wal_checkpoint(TRUNCATE)")?;',
    '            .execute_batch("PRAGMA wal_checkpoint(TRUNCATE)")?;\n        if std::env::var_os("TINE_DIAGNOSE_543_VFS").is_some() { sql543_vfs_dump(); }')
with file.open('a', encoding='utf-8') as output:
    output.write(pathlib.Path('scripts/diagnose-543-vfs.rs').read_text(encoding='utf-8'))
patch(file, '        Ok(Self { connection })\n    }\n\n    pub fn open_read_only',
    '        if std::env::var_os("TINE_DIAGNOSE_543_NO_AUTOCHECKPOINT").is_some() {\n'
    '            connection.pragma_update(None, "wal_autocheckpoint", 0)?;\n'
    '        }\n        if let Ok(kib) = std::env::var("TINE_DIAGNOSE_543_CACHE_KIB") {\n'
    '            connection.pragma_update(None, "cache_size", -kib.parse::<i64>().unwrap())?;\n'
    '        }\n        Ok(Self { connection })\n    }\n\n    pub fn open_read_only')
patch(file, '        let instrumentation = sqlite_materialization::apply_graph_projection_rows(',
    '        let sql543 = std::time::Instant::now();\n        let instrumentation = sqlite_materialization::apply_graph_projection_rows(')
patch(file, '        sqlite_materialization::replace_graph_projection_reference_facts(',
    timer('graph_rows') + '        sqlite_materialization::replace_graph_projection_reference_facts(')
patch(file, '        if let Some(portable_paths) = portable_paths {',
    timer('reference_facts') + '        if let Some(portable_paths) = portable_paths {')
patch(file, '        transaction.commit()?;\n        Ok(instrumentation)',
    timer('revision_and_order') + '        transaction.commit()?;' + timer('commit') + '        let _ = sql543;\n        Ok(instrumentation)')

file = target / 'src/sqlite_materialization.rs'
patch(file, '    let old_fts = load_fts_source_rows(transaction, &affected_pages)?;',
    '    let sql543 = std::time::Instant::now();\n    let old_fts = load_fts_source_rows(transaction, &affected_pages)?;' + timer('load_old_fts'))
patch(file, '    for page in replacements {\n        insert_page(transaction, page)?;\n    }',
    timer('cleanup') + '    for (index, page) in replacements.iter().enumerate() {\n'
    '        if index % 1000 == 0 { eprintln!("SQL543 insert_progress={index}/{} elapsed={:?}", replacements.len(), sql543.elapsed()); sql543_stats(transaction, "insert_progress"); }\n'
    '        insert_page(transaction, page)?;\n    }' + timer('insert_pages') + '    sql543_stats(transaction, "insert_done");\n')
patch(file, '        fts_instrumentation,\n    )?;\n    Ok(instrumentation)',
    '        fts_instrumentation,\n    )?;' + timer('reconcile_fts') + '    let _ = sql543;\n    Ok(instrumentation)')
patch(file, '    Ok(transaction.prepare_cached(sql)?.execute(parameters)?)',
    '    let start = std::time::Instant::now();\n'
    '    let result = transaction.prepare_cached(sql)?.execute(parameters);\n'
    '    sql543_record(sql, start.elapsed());\n    Ok(result?)')
with file.open('a', encoding='utf-8') as output:
    output.write(pathlib.Path('scripts/diagnose-543-sql-helpers.rs').read_text(encoding='utf-8'))

config = pathlib.Path('.cargo/config.toml')
config.parent.mkdir(exist_ok=True)
assert not config.exists()
config.write_text('[patch."https://github.com/martinkoutecky/tine-storage"]\ntine-storage = { path = "vendor/tine-storage-543" }\n', encoding='utf-8')
subprocess.run(['cargo', 'metadata', '--offline', '--format-version=1'], stdout=subprocess.DEVNULL, check=True)
print('SQL543 overlay ready:', target)
