/** The one forward table from `Backend` graph questions to browser core
 * commands (ADR 0081). The published export and the browser mock both spread
 * it, so a question the core answers has exactly one browser adapter. */
import type { Backend } from "./backendInterface";
import type { ConfigProblem } from "./backendTypes";
import type { GraphRead } from "./publishedGraphRpc";
import type { GraphMeta, PageRead, PageReadSource, RefGroup } from "./types";

/** The graph's effective metadata and config read problem, as the desktop `load_graph` answers them. */
export async function graphIdentity(graphRead: GraphRead) {
  const read = async <T>(...call: Parameters<GraphRead>[0]): Promise<T> => JSON.parse(await graphRead(call));
  const [meta, config_problem] = await Promise.all([
    read<GraphMeta>("graph_meta_json"),
    read<ConfigProblem | null>("config_problem_json"),
  ]);
  return { meta, config_problem };
}

/** Reads every browser graph answers, the read-only export included. */
export function graphReads(graphRead: GraphRead) {
  const read = async <T>(...call: Parameters<GraphRead>[0]): Promise<T> => JSON.parse(await graphRead(call));
  return {
    pageInventory: () => read("page_inventory_json"),
    pageModified: () => read("page_modified_json"),
    pageBacklinkCounts: () => read("page_backlink_counts_json"),
    resolvePage: (name, kind) => read("resolve_page_json", name, kind === "journal"),
    pageAliasGroup: name => read("page_alias_group_json", name),
    getPage: (name, kind) => read("get_page_json", name, kind === "journal"),
    async getPageByPath(path, source) {
      const answer = await read<{ page: PageRead | null; source: PageReadSource | null }>("get_page_by_path_json", path);
      source?.(answer.source);
      return answer.page;
    },
    journalFeedPage: (limit, before, waitForReferences = false, through = null) =>
      read("journal_feed_page_json", limit, before === null ? undefined : BigInt(before),
        waitForReferences, through === null ? undefined : BigInt(through)),
    journalContentDays: () => read("journal_content_days_json"),
    pageIcons: names => read("page_icons_json", JSON.stringify(names)),
    getBacklinks: name => read("get_backlinks_json", name),
    getBacklinkFilterContext: (name, targets) =>
      read("get_backlink_filter_context_json", name, JSON.stringify(targets)),
    getUnlinkedRefs: name => read("get_unlinked_refs_json", name),
    getBlockRefCounts: () => read("get_block_ref_counts_json"),
    getBlockReferrers: uuid => read("get_block_referrers_json", uuid),
    async resolveBlock(uuid) {
      const [group] = await read<(RefGroup | null)[]>("resolve_blocks_json", JSON.stringify([uuid]));
      return group;
    },
    resolveBlocks: uuids => read("resolve_blocks_json", JSON.stringify(uuids)),
    previewBlock: (uuid, maxNodes) => read("preview_block_json", uuid, maxNodes),
    search: (query, limit) => read("search_json", query, limit),
    quickSwitch: (query, limit, _editor, transliteration = false) =>
      read("capture_quick_switch_json", query, limit, transliteration),
    captureQuickSwitch: (query, limit, transliteration = false) =>
      read("capture_quick_switch_json", query, limit, transliteration),
    queryFacets: scope => read("query_facets_json", JSON.stringify(scope)),
    parseQuery: async (text, dialect, properties, location) =>
      JSON.parse(await graphRead(["parse_query_json", text, dialect, JSON.stringify(properties)], location)),
    queryRun: (query, view, context) =>
      read("query_run_json", JSON.stringify(query), JSON.stringify(view), JSON.stringify(context)),
    runGraphSearch: (source, pageLimit, blockLimit, _lane, explain, scope, pageMatchScope, views, transliteration) =>
      read("run_graph_search_json", source, pageLimit, blockLimit, JSON.stringify({
        explain, scope, pageMatchScope, pageView: views?.page, blockView: views?.block, transliteration,
      })),
  } satisfies Partial<Backend>;
}

/** Store answers a read-only export does not offer (its refusals stay): the
 * templates, Copy/Export subtrees, tagged pages and a query's explain-empty. */
export function graphStoreReads(graphRead: GraphRead) {
  const read = async <T>(...call: Parameters<GraphRead>[0]): Promise<T> => JSON.parse(await graphRead(call));
  return {
    listTemplateNames: () => read("list_template_names_json"),
    listTemplates: name => read("list_templates_json", name),
    exportQuerySubtrees: specs => read("export_query_subtrees_json", JSON.stringify(specs)),
    taggedPages: name => read("tagged_pages_json", name),
    queryExplainEmpty: (query, view, context) =>
      read("query_explain_empty_json", JSON.stringify(query), JSON.stringify(view), JSON.stringify(context)),
  } satisfies Partial<Backend>;
}
