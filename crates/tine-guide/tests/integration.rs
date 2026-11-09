//! Guide-content assertions that need the Guide and the engine together: each
//! checks that what a Guide page tells the user is what tine-core does. They
//! moved here with the Guide so tine-core's tests no longer read its templates.

#[path = "onboarding_search.rs"]
mod onboarding_search;
#[path = "query_guide.rs"]
mod query_guide;
