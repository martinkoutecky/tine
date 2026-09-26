//! Unit test adapter for the public graph-config client.

use std::io;

use crate::model::Graph;
use tine_graph_features::config;

pub(crate) trait ConfigClient {
    fn set_favorites(&self, names: &[String]) -> io::Result<()>;
    fn set_preferred_workflow(&self, workflow: &str) -> io::Result<()>;
    fn set_default_journal_template(&self, name: Option<&str>) -> io::Result<()>;
    fn set_start_of_week(&self, n: u32) -> io::Result<()>;
    fn set_doc_mode_enter_for_new_block(&self, enabled: bool) -> io::Result<()>;
    fn set_logical_outdenting(&self, enabled: bool) -> io::Result<()>;
}

impl ConfigClient for Graph {
    fn set_favorites(&self, names: &[String]) -> io::Result<()> {
        config::set_favorites(&open(self), names)
    }

    fn set_preferred_workflow(&self, workflow: &str) -> io::Result<()> {
        config::set_preferred_workflow(&open(self), workflow)
    }

    fn set_default_journal_template(&self, name: Option<&str>) -> io::Result<()> {
        config::set_default_journal_template(&open(self), name)
    }

    fn set_start_of_week(&self, n: u32) -> io::Result<()> {
        config::set_start_of_week(&open(self), n)
    }

    fn set_doc_mode_enter_for_new_block(&self, enabled: bool) -> io::Result<()> {
        config::set_doc_mode_enter_for_new_block(&open(self), enabled)
    }

    fn set_logical_outdenting(&self, enabled: bool) -> io::Result<()> {
        config::set_logical_outdenting(&open(self), enabled)
    }
}

fn open(graph: &Graph) -> tine_store::Store {
    tine_store::Store::open(&graph.root, Default::default())
        .unwrap()
        .0
}
