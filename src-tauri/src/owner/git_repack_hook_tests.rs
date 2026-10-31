//! One-shot substitutions at late repack boundaries, compiled only in tests.
type Hook = (&'static str, Box<dyn FnOnce()>);
thread_local! {
    static HOOK: std::cell::RefCell<Option<Hook>> = const { std::cell::RefCell::new(None) };
}

pub(super) fn install(boundary: &'static str, action: impl FnOnce() + 'static) {
    HOOK.with(|hook| *hook.borrow_mut() = Some((boundary, Box::new(action))));
}

pub(super) fn at(boundary: &str) {
    let action = HOOK.with(|hook| {
        let mut hook = hook.borrow_mut();
        if hook.as_ref().is_some_and(|(name, _)| *name == boundary) {
            hook.take().map(|(_, action)| action)
        } else {
            None
        }
    });
    if let Some(action) = action {
        action();
    }
}
