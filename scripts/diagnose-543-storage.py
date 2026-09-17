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
    return f'\n        eprintln!("SQL543 {label}={{:?}}", sql543.elapsed());\n        let sql543 = std::time::Instant::now();\n'

file = target / 'src/sqlite_graph_projection.rs'
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
    timer('cleanup') + '    for page in replacements {\n        insert_page(transaction, page)?;\n    }' + timer('insert_pages'))
patch(file, '        fts_instrumentation,\n    )?;\n    Ok(instrumentation)',
    '        fts_instrumentation,\n    )?;' + timer('reconcile_fts') + '    let _ = sql543;\n    Ok(instrumentation)')

config = pathlib.Path('.cargo/config.toml')
config.parent.mkdir(exist_ok=True)
assert not config.exists()
config.write_text('[patch."https://github.com/martinkoutecky/tine-storage"]\ntine-storage = { path = "vendor/tine-storage-543" }\n', encoding='utf-8')
subprocess.run(['cargo', 'metadata', '--offline', '--format-version=1'], stdout=subprocess.DEVNULL, check=True)
print('SQL543 overlay ready:', target)
