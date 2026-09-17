// Appended only to the disposable CI dependency overlay.
thread_local! {
    static SQL543_COSTS: std::cell::RefCell<std::collections::HashMap<String, (u64, std::time::Duration)>> = Default::default();
}
fn sql543_record(sql: &str, elapsed: std::time::Duration) {
    SQL543_COSTS.with(|costs| {
        let mut costs = costs.borrow_mut();
        if let Some(cost) = costs.get_mut(sql) { cost.0 += 1; cost.1 += elapsed; }
        else { costs.insert(sql.to_owned(), (1, elapsed)); }
    });
}
fn sql543_stats(connection: &Connection, label: &str) {
    let mut stats = Vec::new();
    for (name, operation) in [("cache_bytes", 1), ("cache_hit", 7), ("cache_miss", 8), ("cache_write", 9), ("cache_spill", 12)] {
        let mut current = 0;
        let mut high = 0;
        // The connection remains owned by this thread; diagnostic counters only.
        let result = unsafe { rusqlite::ffi::sqlite3_db_status(connection.handle(), operation, &mut current, &mut high, 0) };
        stats.push(format!("{name}={current} rc={result}"));
    }
    eprintln!("SQL543 CACHE {label} {}", stats.join(" "));
    SQL543_COSTS.with(|costs| {
        let costs = costs.borrow();
        let mut rows = costs.iter().collect::<Vec<_>>();
        rows.sort_by_key(|(_, (_, elapsed))| std::cmp::Reverse(*elapsed));
        for (sql, (count, elapsed)) in rows.into_iter().take(16) {
            let label = sql.split_whitespace().collect::<Vec<_>>().join(" ");
            eprintln!("SQL543 ROW_COST count={count} elapsed={elapsed:?} sql={}", &label[..label.len().min(140)]);
        }
    });
}
