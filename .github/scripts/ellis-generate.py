#!/usr/bin/env python3
"""Generate an Ellis-shaped synthetic Logseq Markdown graph (GH #623).

Input : nodes.csv, edges.csv, totals.txt (anonymous shape export; this directory)
Output: target/ellis-graph/ (repository-relative default; override with --out)
        target/ellis-graph-manifest.json (node id -> name/file, outside the graph)

Stdlib only, deterministic (SEED). All text is synthetic: nothing is taken from the
original graph (the export carries no text).  Assets are SPARSE files (truncate) so they
cost no disk; the manifest records that.

Usage: python3 .github/scripts/ellis-generate.py [--out DIR] [--seed N]
"""
import argparse, csv, collections, datetime, json, math, os, random, sys, uuid

HERE = os.path.dirname(os.path.abspath(__file__))
parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("--out", default=os.path.abspath(os.path.join(HERE, "../../target/ellis-graph")))
parser.add_argument("--seed", type=int, default=623)
args = parser.parse_args()
OUT = args.out
SEED = args.seed
MANIFEST = os.path.join(os.path.dirname(OUT.rstrip("/")), os.path.basename(OUT.rstrip("/")) + "-manifest.json")

rng = random.Random(SEED)

# ---------------------------------------------------------------- input
NODES = {}
for r in csv.DictReader(open(os.path.join(HERE, "nodes.csv"))):
    NODES[int(r["id"])] = dict(kind=r["kind"], bytes=int(r["bytes"]), blocks=int(r["blocks"]),
                               depth=int(r["max_depth"]), props=int(r["properties"]),
                               ids=int(r["block_ids"]), crlf=int(r["crlf"]))
EDGES = [(int(r["source"]), int(r["target"]), r["kind"], int(r["count"]))
         for r in csv.DictReader(open(os.path.join(HERE, "edges.csv")))]
TOT = [list(map(int, l.split())) for l in open(os.path.join(HERE, "totals.txt")) if l.strip()]
ASSET_N, ASSET_BYTES = TOT[0]
BAK_N, BAK_BYTES = TOT[1]
NONASCII_TOTAL = TOT[3][0]
CONFLICT_N = TOT[5][0]
QUERY_MACROS, EMBED_MACROS, OTHER_MACROS = TOT[6][0], TOT[7][0], TOT[8][0]

# ---------------------------------------------------------------- naming
LAT = ("amber birch cedar delta ember fjord garnet harbor indigo juniper kestrel lantern meadow nectar "
       "orchid pewter quartz russet sable tundra umber velvet willow xenon yarrow zephyr alder basalt "
       "cobalt dune estuary flint glacier heron iris jasper kelp lichen marsh nimbus onyx pine quill "
       "reef slate thistle upland vale wren yew zinc anchor beacon canal drift eddy forge grove haven "
       "isle jetty knoll ledge mesa nook oasis prism ridge shoal trail").split()
CJK_POOL = [chr(c) for c in rng.sample(range(0x4E00, 0x4E00 + 3500), 2500)]
nonascii_frac = None  # set below

nodes_nonjournal = [i for i, n in NODES.items() if n["kind"] != "journal"]
q = NONASCII_TOTAL / (len(nodes_nonjournal) - 0 + ASSET_N + BAK_N - 0)
# number of non-ASCII pages among non-journal nodes (files + missing share the same fraction)
n_files_nonj = sum(1 for i in nodes_nonjournal if NODES[i]["kind"] == "page")
pages_na = round(q * n_files_nonj)
bak_na = round(q * BAK_N)
assets_na = NONASCII_TOTAL - pages_na - bak_na
page_frac = pages_na / n_files_nonj  # applied to file-backed pages AND missing nodes


def rand_name(r, nonascii):
    if nonascii:
        n = r.choice([2, 2, 3, 3, 3, 4, 4, 5, 6, 8])
        s = "".join(r.choice(CJK_POOL) for _ in range(n))
        if r.random() < 0.25:
            s += str(r.randrange(2, 99))
        if r.random() < 0.15:
            s = r.choice(LAT).capitalize() + " " + s
        return s
    k = r.choice([1, 1, 2, 2, 2, 3, 3, 4])
    w = [r.choice(LAT) for _ in range(k)]
    s = " ".join(w) if r.random() < 0.65 else "-".join(w)
    if r.random() < 0.55:
        s += " " + format(r.randrange(36 ** 3), "x")
    return s.capitalize() if r.random() < 0.5 else s


ids_sorted = sorted(NODES)
NAME = {}
SEEN = set()
# journals: distinct dates
jids = [i for i in ids_sorted if NODES[i]["kind"] == "journal"]
d0 = datetime.date(2019, 6, 1)
span = (datetime.date(2025, 9, 30) - d0).days
dates = sorted(rng.sample(range(span + 1), len(jids)))
JDATE = {}


def ordinal(d):
    return str(d) + ("th" if 11 <= d % 100 <= 13 else {1: "st", 2: "nd", 3: "rd"}.get(d % 10, "th"))


rng.shuffle(jids)
for jid, off in zip(jids, dates):
    dt = d0 + datetime.timedelta(days=off)
    JDATE[jid] = dt
    NAME[jid] = "%s %s, %d" % (dt.strftime("%b"), ordinal(dt.day), dt.year)
    SEEN.add(NAME[jid].lower())
others = [i for i in ids_sorted if NODES[i]["kind"] != "journal"]
na_flags = [True] * round(page_frac * len(others)) + [False] * (len(others) - round(page_frac * len(others)))
# files exactly pages_na non-ASCII
rng.shuffle(others)
files_o = [i for i in others if NODES[i]["kind"] == "page"]
miss_o = [i for i in others if NODES[i]["kind"] == "missing"]
flag = {}
for i, f in zip(files_o, [True] * pages_na + [False] * (len(files_o) - pages_na)):
    flag[i] = f
mna = round(page_frac * len(miss_o))
for i, f in zip(miss_o, [True] * mna + [False] * (len(miss_o) - mna)):
    flag[i] = f
# namespace children: pick names after parents exist
NS_EDGE = {s: t for s, t, k, c in EDGES if k == "namespace"}
nr = random.Random(SEED + 1)
for i in others:
    if i in NS_EDGE:
        continue
    while True:
        nm = rand_name(nr, flag[i])
        if nm.lower() not in SEEN:
            SEEN.add(nm.lower())
            NAME[i] = nm
            break
for s, t in NS_EDGE.items():
    while True:
        nm = NAME[t] + "/" + rand_name(nr, flag[s])
        if nm.lower() not in SEEN:
            SEEN.add(nm.lower())
            NAME[s] = nm
            break


def page_filename(i):
    n = NODES[i]
    if n["kind"] == "journal":
        return "journals/%s.md" % JDATE[i].strftime("%Y_%m_%d")
    return "pages/%s.md" % NAME[i].replace("/", "___")


# ---------------------------------------------------------------- block ids
ID_UUIDS = {}
ur = random.Random(SEED + 2)
USED_UUID = set()
for i in ids_sorted:
    n = NODES[i]
    if n["kind"] != "missing" and n["ids"] > 0:
        lst = []
        for _ in range(min(n["ids"], max(n["blocks"], 1))):
            while True:
                u = str(uuid.UUID(int=ur.getrandbits(128), version=4))
                if u not in USED_UUID:
                    USED_UUID.add(u)
                    lst.append(u)
                    break
        ID_UUIDS[i] = lst

# ---------------------------------------------------------------- text pools
POOL_LAT = " ".join(
    rng.choice(LAT) + rng.choice(["", "", "s", "ing", "ed", "ly", "tion"]) +
    rng.choice(["", "", "", ",", "."]) for _ in range(700000))
POOL_LAT = POOL_LAT[:4_000_000]
POOL_CJK = "".join(rng.choice(CJK_POOL) for _ in range(900000))


def filler(r, L, cjk_frac):
    """Return text of exactly L UTF-8 bytes, no leading/trailing space, no markup."""
    if L <= 0:
        return ""
    if L < 4:
        return "".join(r.choice("abcdefghijklmnopqrstuvwxyz") for _ in range(L))
    nc = int(L * cjk_frac / 3)
    # randomly decide to include CJK at all for short strings
    if nc > 0 and L < 12 and r.random() < 0.5:
        nc = 0
    nl = L - 3 * nc
    out = []
    if nc:
        # a latin lead part and a latin tail part, cjk in the middle
        a = r.randint(0, nl) if nl else 0
        parts = [("l", a), ("c", nc), ("l", nl - a)]
    else:
        parts = [("l", nl)]
    for kind, n in parts:
        if n <= 0:
            continue
        if kind == "c":
            o = r.randrange(len(POOL_CJK) - min(n, len(POOL_CJK)) + 1)
            s = POOL_CJK[o:o + n] if n <= len(POOL_CJK) else (POOL_CJK * (n // len(POOL_CJK) + 1))[:n]
            out.append(s)
        else:
            if n <= len(POOL_LAT):
                o = r.randrange(len(POOL_LAT) - n + 1)
                s = POOL_LAT[o:o + n]
            else:
                s = (POOL_LAT * (n // len(POOL_LAT) + 1))[:n]
            out.append(s)
    s = "".join(out)
    # sanitize ends / interior double-space-neutral; replace edge spaces with letters (same bytes)
    if s[0] in " ,.":
        s = "a" + s[1:]
    if s[-1] in " ,.":
        s = s[:-1] + "a"
    return s


# ---------------------------------------------------------------- edges -> atoms per source
BY_SRC = collections.defaultdict(list)
for s, t, k, c in EDGES:
    BY_SRC[s].append((t, k, c))

PROP_KEYS = ["related", "source", "author", "see-also", "type", "project", "parent", "topic"]
FREE_KEYS = ["status", "priority", "rating", "created", "kind", "area", "stage", "owner", "due", "size"]


def ref(i, r, lower_ok=True):
    nm = NAME[i]
    if lower_ok and r.random() < 0.08 and nm.isascii():
        nm = nm.lower()
    return nm


def tag_atom(i, r):
    nm = NAME[i]
    simple = (" " not in nm) and ("/" not in nm) and (all(ch.isalnum() or ch in "-_" for ch in nm))
    if simple and r.random() < 0.6:
        return "#" + nm
    return "#[[" + nm + "]]"


STATS = collections.Counter()


def build_page(i):
    n = NODES[i]
    r = random.Random("%d-%d" % (SEED, i))
    S, B, D, P, I = n["bytes"], n["blocks"], n["depth"], n["props"], n["ids"]
    eol = "\r\n" if n["crlf"] else "\n"
    if S == 0:
        return b""
    cjk_frac = 0.7 if (NAME[i] and not NAME[i].isascii()) else 0.06
    if r.random() < 0.15:
        cjk_frac = 1 - cjk_frac
    ed = BY_SRC.get(i, [])
    # ---- structure of blocks
    ds = []
    if B > 0:
        ds = [0] * B
        chain = None
        if D > 0:
            Dd = min(D, B - 1)
            if Dd < D:
                STATS["depth_clamped"] += 1
            s0 = r.randint(0, B - Dd - 1)
            chain = (s0, Dd)
        else:
            Dd = 0
        pd = 0.18 if Dd >= 2 else 0.3
        cur = 0
        for b in range(B):
            if chain and chain[0] <= b <= chain[0] + chain[1]:
                cur = b - chain[0]
            elif b == 0:
                cur = 0
            elif Dd == 0:
                cur = 0
            else:
                x = r.random()
                if x < pd:
                    cur = min(cur + 1, Dd)
                elif x < 0.5:
                    pass
                else:
                    cur = max(0, cur - 1 - int(r.expovariate(1.2)))
            ds[b] = cur
    # ---- ids placement
    id_blocks = {}
    if B > 0 and I > 0:
        uu = ID_UUIDS[i]
        if len(uu) < I:
            STATS["ids_clamped_by_blocks"] += 1
        for bi, u in zip(r.sample(range(B), len(uu)), uu):
            id_blocks[bi] = u
    top_ids = []
    if B == 0 and I > 0:
        top_ids = ["id:: " + u for u in ID_UUIDS[i]]
    nid = len(id_blocks) + len(top_ids)
    # ---- atoms
    atoms = []          # inline atoms (strings)
    alias_names = []    # alias values
    prop_atoms = []     # property-edge values "[[x]]"
    tags_rows = []      # candidates for tags:: line
    for t, k, c in ed:
        if k == "link":
            atoms += ["[[%s]]" % ref(t, r) for _ in range(c)]
        elif k == "tag":
            if c == 1 and r.random() < 0.35:
                tags_rows.append(t)
            else:
                atoms += [tag_atom(t, r) for _ in range(c)]
        elif k == "alias":
            alias_names += [NAME[t]] * c
        elif k == "property":
            prop_atoms += ["[[%s]]" % NAME[t]] * c
        elif k == "blockref":
            tu = ID_UUIDS.get(t)
            if not tu:
                STATS["blockref_target_no_id"] += 1
                continue
            atoms += ["((%s))" % r.choice(tu) for _ in range(c)]
        elif k == "embed":
            atoms += ["{{embed [[%s]]}}" % NAME[t] for _ in range(c)]
        elif k == "query":
            atoms += ["{{query (page [[%s]])}}" % NAME[t] for _ in range(c)]
        elif k == "namespace":
            pass  # realized by file name
    # macros without edges are assigned later via EXTRA[i]
    atoms += EXTRA.get(i, [])
    # ---- property lines: P counts every `key:: value` line incl. id::
    avail = P - nid
    lines_page = []   # bare page-level property lines (top of file)
    needed = []
    if alias_names:
        needed.append("alias")
    if prop_atoms:
        needed.append("prop")
    if avail < len(needed):
        STATS["props_forced_extra"] += len(needed) - max(avail, 0)
        avail = len(needed)
    if alias_names:
        if r.random() < 0.5:
            lines_page.append("alias:: " + ", ".join(alias_names))
        else:
            lines_page.append("alias:: " + ", ".join("[[%s]]" % x for x in alias_names))
        avail -= 1
    if prop_atoms:
        nl = max(1, min(len(prop_atoms), avail - (1 if tags_rows else 0)))
        if avail - nl < 0:
            nl = max(1, avail)
        chunks = [[] for _ in range(nl)]
        for j, a in enumerate(prop_atoms):
            chunks[j % nl].append(a)
        keys = r.sample(PROP_KEYS, min(nl, len(PROP_KEYS)))
        for j, ch in enumerate(chunks):
            lines_page.append("%s:: %s" % (keys[j % len(keys)] + ("" if j < len(keys) else str(j)), ", ".join(ch)))
        avail -= nl
    if tags_rows:
        if avail >= 1:
            lines_page.append("tags:: " + ", ".join("[[%s]]" % NAME[t] for t in tags_rows))
            avail -= 1
        else:
            atoms += [tag_atom(t, r) for t in tags_rows]
    free = max(avail, 0)
    # free generic property lines
    free_by_block = collections.defaultdict(list)
    top_free = []
    if B > 0:
        for j in range(free):
            b = r.randrange(B)
            free_by_block[b].append("%s:: %s" % (r.choice(FREE_KEYS), r.choice(["draft", "done", "high", "3", "2024-01-05", "alpha", "beta", "x1"])))
    else:
        top_free = ["%s:: %s" % (r.choice(FREE_KEYS), r.choice(["draft", "done", "high", "3", "alpha"])) for _ in range(free)]
    # ---- assemble block records
    if B > 0:
        text_atoms = [[] for _ in range(B)]
        for a in atoms:
            text_atoms[r.randrange(B)].append(a)
        blocks = []
        for b in range(B):
            props = []
            if b in id_blocks:
                props.append("id:: " + id_blocks[b])
            props += free_by_block.get(b, [])
            blocks.append(dict(d=ds[b], atoms=text_atoms[b], props=props))
        head = lines_page + top_free
        # fixed bytes
        ew = len(eol)
        fixed = 0
        for h in head:
            fixed += len(h.encode()) + ew
        for bl in blocks:
            ind = bl["d"]
            fixed += ind + 2 + ew
            for p in bl["props"]:
                fixed += ind + 2 + len(p.encode()) + ew
            if bl["atoms"]:
                fixed += sum(len(a.encode()) for a in bl["atoms"]) + (len(bl["atoms"]) - 1)
        # blocks that will carry filler pay one separator if they also have atoms
        nb = B
        F = S - fixed
        withatoms = sum(1 for bl in blocks if bl["atoms"])
        if F >= nb + withatoms:
            F -= withatoms
            order = list(range(nb))
            w = [r.lognormvariate(0, 1.0) for _ in range(nb)]
            tot = sum(w)
            rem = F - nb
            fl = [1 + int(rem * x / tot) for x in w]
            fl[max(range(nb), key=lambda j: w[j])] += F - sum(fl)
        else:
            # not enough budget for a filler char everywhere: spend what we have on random blocks
            STATS["bytes_tight"] += 1
            fl = [0] * nb
            F2 = max(F, 0)
            for j in r.sample(range(nb), min(nb, F2)):
                fl[j] = 1
            if F < 0:
                STATS["bytes_overshoot"] += 1
        # render (texts cached so the exact-size correction only regenerates one block)
        def render(fl_):
            out = list(head)
            for bi, bl in enumerate(blocks):
                ind = "\t" * bl["d"]
                txt = TX[bi] if fl_[bi] == FLC[bi] else filler(random.Random("%d-%d-%d" % (SEED, i, bi)), fl_[bi], cjk_frac)
                if bl["atoms"]:
                    at = " ".join(bl["atoms"])
                    txt = (txt + " " + at) if txt else at
                out.append(ind + "- " + txt)
                for p in bl["props"]:
                    out.append(ind + "  " + p)
            return (eol.join(out) + eol).encode()
        TX = [filler(random.Random("%d-%d-%d" % (SEED, i, bi)), fl[bi], cjk_frac) for bi in range(nb)]
        FLC = list(fl)
        data = render(fl)
        diff = S - len(data)
        if diff != 0:
            j = max(range(nb), key=lambda x: fl[x])
            if fl[j] + diff >= 0:
                fl[j] += diff
                data = render(fl)
        if len(data) != S:
            STATS["size_inexact"] += 1
        return data
    # ---- B == 0: bare paragraph file
    head = lines_page + top_free + top_ids
    ew = len(eol)
    if not atoms and not head and S <= ew:
        return (eol if S == ew else "x" * S).encode()
    nlines = max(1, min(20000, S // 90))
    atoms_by_line = [[] for _ in range(nlines)]
    for a in atoms:
        atoms_by_line[r.randrange(nlines)].append(a)
    fixed = sum(len(h.encode()) + ew for h in head) + nlines * ew
    for al in atoms_by_line:
        if al:
            fixed += sum(len(a.encode()) for a in al) + (len(al) - 1) + 1
    F = S - fixed
    if F < nlines:
        nlines2 = max(1, min(nlines, max(F, 1)))
        nlines = nlines2
    w = [r.lognormvariate(0, 0.7) for _ in range(nlines)]
    tot = sum(w)
    atoms_by_line = [[] for _ in range(nlines)]
    for a in atoms:
        atoms_by_line[r.randrange(nlines)].append(a)
    fixed = sum(len(h.encode()) + ew for h in head) + nlines * ew
    for al in atoms_by_line:
        if al:
            fixed += sum(len(a.encode()) for a in al) + (len(al) - 1) + 1
    F = S - fixed
    if F < nlines:
        STATS["bytes_overshoot_plain"] += 1
        fl = [1] * nlines
    else:
        rem = F - nlines
        fl = [1 + int(rem * x / tot) for x in w]
        fl[max(range(nlines), key=lambda j: w[j])] += F - sum(fl)
    out = list(head)
    for li in range(nlines):
        txt = filler(r, fl[li], cjk_frac)
        if atoms_by_line[li]:
            txt = txt + " " + " ".join(atoms_by_line[li])
        out.append(txt)
    return (eol.join(out) + eol).encode()


EXTRA = {}


# ---------------------------------------------------------------- main
def main():
    global EXTRA
    mr = random.Random(SEED + 3)
    # macros without an edge row: queries (22 total incl. the 1 with a page predicate), embeds, others
    hosts = [i for i in ids_sorted if NODES[i]["kind"] != "missing" and NODES[i]["bytes"] >= 400 and NODES[i]["blocks"] > 0]
    def put(a):
        EXTRA.setdefault(mr.choice(hosts), []).append(a)
    for k in range(QUERY_MACROS - sum(1 for e in EDGES if e[2] == "query")):
        put(mr.choice(["{{query (and (task TODO DOING) (priority a))}}", "{{query (task NOW LATER)}}",
                       "{{query (and (task DONE) (between -7d today))}}", "{{query (priority b)}}"]))
    emb_edges = sum(e[3] for e in EDGES if e[2] == "embed")
    for k in range(EMBED_MACROS - emb_edges):
        put("{{embed https://example.invalid/e/%d}}" % k)
    for k in range(OTHER_MACROS):
        put(mr.choice(["{{video https://example.invalid/clip/%d}}", "{{cloze item %d}}", "{{renderer :todomaster, %d}}"]) % k)
    os.makedirs(OUT, exist_ok=True)
    for d in ("pages", "journals", "assets", "logseq/bak/pages"):
        os.makedirs(os.path.join(OUT, d), exist_ok=True)
    with open(os.path.join(OUT, "logseq/config.edn"), "w") as f:
        f.write("{:file/name-format :triple-lowbar}\n")
    manifest = dict(seed=SEED, nodes={}, sparse_assets=True)
    tm = random.Random(SEED + 4)
    t_lo = datetime.datetime(2021, 1, 1).timestamp()
    t_hi = datetime.datetime(2025, 9, 30).timestamp()
    nfiles = 0
    for i in ids_sorted:
        n = NODES[i]
        ent = dict(kind=n["kind"], name=NAME[i])
        if n["kind"] != "missing":
            fn = page_filename(i)
            data = build_page(i)
            p = os.path.join(OUT, fn)
            with open(p, "wb") as f:
                f.write(data)
            if n["kind"] == "journal":
                ts = datetime.datetime.combine(JDATE[i], datetime.time(20, 0)).timestamp()
            else:
                ts = tm.uniform(t_lo, t_hi)
            os.utime(p, (ts, ts))
            ent["file"] = fn
            nfiles += 1
        manifest["nodes"][str(i)] = ent
    # --- logseq/bak: BAK_N real files, BAK_BYTES total
    br = random.Random(SEED + 5)
    ws = [br.lognormvariate(0, 1.4) for _ in range(BAK_N)]
    sizes = [max(50, int(BAK_BYTES * x / sum(ws))) for x in ws]
    sizes[0] += BAK_BYTES - sum(sizes)
    bak_flags = [True] * bak_na + [False] * (BAK_N - bak_na)
    br.shuffle(bak_flags)
    conflict_idx = set(br.sample(range(BAK_N), CONFLICT_N))
    for k in range(BAK_N):
        base = rand_name(br, bak_flags[k]).replace("/", "_") + " %d" % k
        d = os.path.join(OUT, "logseq/bak/pages", base)
        os.makedirs(d, exist_ok=True)
        ts = "2024-%02d-%02dT10_%02d_00.000Z" % (br.randint(1, 12), br.randint(1, 28), br.randint(0, 59))
        nm = ts + (".sync-conflict-20240115-101112-ABCDEFG" if k in conflict_idx else ".Desktop") + ".md"
        s = sizes[k]
        txt = ("- " + filler(br, s - 3, 0.3) + "\n") if s > 3 else "x" * s
        with open(os.path.join(d, nm), "wb") as f:
            f.write(txt.encode())
    # --- assets: SPARSE files
    ar = random.Random(SEED + 6)
    ws = [ar.lognormvariate(0, 1.8) for _ in range(ASSET_N)]
    sizes = [max(1024, int(ASSET_BYTES * x / sum(ws))) for x in ws]
    sizes[sizes.index(min(sizes))] += ASSET_BYTES - sum(sizes)
    na_flags = [True] * assets_na + [False] * (ASSET_N - assets_na)
    ar.shuffle(na_flags)
    exts = [".png", ".png", ".jpg", ".jpg", ".pdf", ".mp4", ".webp", ".gif", ".mp3", ".zip"]
    for k in range(ASSET_N):
        if na_flags[k]:
            nm = "".join(ar.choice(CJK_POOL) for _ in range(ar.randint(2, 6))) + "_%d" % k
        else:
            nm = "image_%d_%d_0" % (1600000000000 + ar.randrange(10 ** 11), k)
        p = os.path.join(OUT, "assets", nm + ar.choice(exts))
        with open(p, "wb") as f:
            f.truncate(sizes[k])
    manifest["assets"] = dict(count=ASSET_N, bytes=ASSET_BYTES, sparse=True,
                              note="assets/* are sparse files (truncate); apparent size 26.7 GB, ~0 disk blocks")
    manifest["stats"] = dict(STATS)
    manifest["nonascii"] = dict(pages=pages_na, bak=bak_na, assets=assets_na, total=NONASCII_TOTAL)
    with open(MANIFEST, "w") as f:
        json.dump(manifest, f, ensure_ascii=False)
    print("files", nfiles, dict(STATS))


main()
