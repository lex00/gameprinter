"""The same six questions on the same graph in DuckDB (SQL and SQL/PGQ),
Kuzu and LadybugDB (Cypher). Prototype for docs/research/graphs.md.

  python bench.py GRAPH_DIR WORK_DIR ENGINE [QUERY]

ENGINE is duckdb, duckpgq, kuzu or ladybug. With no QUERY it loads
GRAPH_DIR's nodes.parquet and edges.parquet into a database file under
WORK_DIR and prints the load time. With a QUERY id it opens that file in
this fresh process, runs the query once (first run) and 10 times more
(warm), and prints JSON: rows, first, warm median, min and max, in ms.
"""

import json
import statistics
import sys
import time
from pathlib import Path

CODE = "('calls','jumps_to','dispatches')"

SQL = {
    # Which clips would a change to MoveNormalEnemy affect?
    "q1_impact": """
        SELECT src FROM edges
        WHERE rel = 'runs' AND dst = 'routine:MoveNormalEnemy' ORDER BY src""",
    # What code backs the goomba's facts: cited routines and all they call.
    "q2_code_behind": f"""
        WITH RECURSIVE r(id) AS (
          SELECT DISTINCT dst FROM edges
          WHERE rel = 'cites' AND src LIKE 'fact:entities.goomba.%'
          UNION
          SELECT e.dst FROM edges e JOIN r ON e.src = r.id WHERE e.rel IN {CODE})
        SELECT id FROM r ORDER BY id""",
    # Which mechanics share a RAM byte, through the code their facts cite?
    "q3_shared_ram": """
        SELECT w.dst AS ram, count(DISTINCT p.dst) AS mechanics
        FROM edges p
        JOIN edges c ON c.src = p.src AND c.rel = 'cites'
        JOIN edges w ON w.src = c.dst AND w.rel = 'writes'
        WHERE p.rel = 'part_of'
        GROUP BY w.dst HAVING count(DISTINCT p.dst) > 1 ORDER BY ram""",
    # From the goomba to its frames, object ID, the routines naming it, what
    # they write, the fields over those bytes, and the clips that run them.
    "q4_asset_walk": """
        WITH s AS (SELECT 'entity:goomba' AS id),
        r AS (SELECT DISTINCT c.src AS id FROM edges a
              JOIN edges n ON n.src = a.dst AND n.rel = 'named'
              JOIN edges c ON c.dst = n.dst AND c.rel = 'uses_const'
              WHERE a.src = 'entity:goomba' AND a.rel = 'has_object_id'),
        out AS (
          SELECT dst AS id FROM edges WHERE src = 'entity:goomba' AND rel IN ('drawn_with','uses_tiles','carries')
          UNION SELECT id FROM r
          UNION SELECT w.dst FROM edges w JOIN r ON w.src = r.id WHERE w.rel = 'writes'
          UNION SELECT m.dst FROM edges w JOIN r ON w.src = r.id
                JOIN edges m ON m.src = w.dst AND m.rel = 'mapped_to' WHERE w.rel = 'writes'
          UNION SELECT k.src FROM edges k JOIN r ON k.dst = r.id WHERE k.rel = 'runs')
        SELECT DISTINCT id FROM out ORDER BY id""",
    # Missing mechanics: routines a clip runs that no fact cites, and that no
    # cited routine reaches.
    "q5_uncited": f"""
        WITH RECURSIVE r(id) AS (
          SELECT DISTINCT dst FROM edges WHERE rel = 'cites'
          UNION
          SELECT e.dst FROM edges e JOIN r ON e.src = r.id WHERE e.rel IN {CODE})
        SELECT DISTINCT dst FROM edges WHERE rel = 'runs'
          AND dst NOT IN (SELECT id FROM r) ORDER BY dst""",
    # Every routine reachable from GameCoreRoutine (the game engine's entry) through calls, jumps and tables.
    "q6_reach": f"""
        WITH RECURSIVE r(id) AS (
          SELECT 'routine:GameCoreRoutine'
          UNION
          SELECT e.dst FROM edges e JOIN r ON e.src = r.id WHERE e.rel IN {CODE})
        SELECT id FROM r WHERE id <> 'routine:GameCoreRoutine' ORDER BY id""",
}

# SQL/PGQ, where it reads naturally. Code edges are their own edge table.
PGQ = {
    "q1_impact": """
        FROM GRAPH_TABLE (gp MATCH (c:node)-[e:runs]->(r:node WHERE r.id = 'routine:MoveNormalEnemy')
          COLUMNS (c.id AS src)) ORDER BY src""",
    "q3_shared_ram": """
        FROM GRAPH_TABLE (gp MATCH (m:node)<-[p:part_of]-(f:node)-[c:cites]->(r:node)-[w:writes]->(b:node)
          COLUMNS (b.id AS ram, m.id AS mech))
        SELECT ram, count(DISTINCT mech) AS mechanics GROUP BY ram HAVING count(DISTINCT mech) > 1 ORDER BY ram""",
    "q6_reach": """
        FROM GRAPH_TABLE (gp MATCH (a:node WHERE a.id = 'routine:GameCoreRoutine')-[e:code]->{1,30}(b:node)
          COLUMNS (b.id AS id))
        SELECT DISTINCT id WHERE id <> 'routine:GameCoreRoutine' ORDER BY id""",
}

CY_CODE = "['calls','jumps_to','dispatches']"
CYPHER = {
    "q1_impact": """
        MATCH (c:Node)-[e:E {rel: 'runs'}]->(r:Node {id: 'routine:MoveNormalEnemy'})
        RETURN c.id AS src ORDER BY src""",
    "q2_code_behind": f"""
        MATCH (f:Node)-[c:E {{rel: 'cites'}}]->(r:Node)
        WHERE f.id STARTS WITH 'fact:entities.goomba.'
        WITH DISTINCT r
        MATCH (r)-[p:E* SHORTEST 1..30 (e, n | WHERE e.rel IN {CY_CODE})]->(x:Node)
        RETURN x.id AS id
        UNION
        MATCH (f:Node)-[c:E {{rel: 'cites'}}]->(r:Node)
        WHERE f.id STARTS WITH 'fact:entities.goomba.'
        RETURN r.id AS id""",
    "q3_shared_ram": """
        MATCH (m:Node)<-[:E {rel: 'part_of'}]-(f:Node)-[:E {rel: 'cites'}]->(r:Node)-[:E {rel: 'writes'}]->(b:Node)
        WITH b.id AS ram, count(DISTINCT m.id) AS mechanics WHERE mechanics > 1
        RETURN ram, mechanics ORDER BY ram""",
    "q4_asset_walk": """
        MATCH (g:Node {id: 'entity:goomba'})-[e:E]->(d:Node)
        WHERE e.rel IN ['drawn_with','uses_tiles','carries'] RETURN d.id AS id
        UNION
        MATCH (:Node {id: 'entity:goomba'})-[:E {rel: 'has_object_id'}]->(:Node)-[:E {rel: 'named'}]->(:Node)<-[:E {rel: 'uses_const'}]-(r:Node)
        RETURN r.id AS id
        UNION
        MATCH (:Node {id: 'entity:goomba'})-[:E {rel: 'has_object_id'}]->(:Node)-[:E {rel: 'named'}]->(:Node)<-[:E {rel: 'uses_const'}]-(r:Node)-[:E {rel: 'writes'}]->(b:Node)
        RETURN b.id AS id
        UNION
        MATCH (:Node {id: 'entity:goomba'})-[:E {rel: 'has_object_id'}]->(:Node)-[:E {rel: 'named'}]->(:Node)<-[:E {rel: 'uses_const'}]-(r:Node)-[:E {rel: 'writes'}]->(:Node)-[:E {rel: 'mapped_to'}]->(f:Node)
        RETURN f.id AS id
        UNION
        MATCH (:Node {id: 'entity:goomba'})-[:E {rel: 'has_object_id'}]->(:Node)-[:E {rel: 'named'}]->(:Node)<-[:E {rel: 'uses_const'}]-(r:Node)<-[:E {rel: 'runs'}]-(c:Node)
        RETURN c.id AS id""",
    "q5_uncited": f"""
        MATCH (:Node)-[:E {{rel: 'cites'}}]->(r:Node)
        WITH DISTINCT r
        OPTIONAL MATCH (r)-[:E* SHORTEST 1..30 (e, n | WHERE e.rel IN {CY_CODE})]->(x:Node)
        WITH collect(DISTINCT x.id) + collect(DISTINCT r.id) AS reached
        MATCH (:Node)-[:E {{rel: 'runs'}}]->(u:Node)
        WHERE NOT list_contains(reached, u.id)
        RETURN DISTINCT u.id AS dst ORDER BY dst""",
    "q6_reach": f"""
        MATCH (a:Node {{id: 'routine:GameCoreRoutine'}})-[:E* SHORTEST 1..30 (e, n | WHERE e.rel IN {CY_CODE})]->(b:Node)
        WHERE b.id <> 'routine:GameCoreRoutine'
        RETURN DISTINCT b.id AS id ORDER BY id""",
}


def ms(t):
    return round(t * 1000, 3)


def duck(work, pgq):
    import duckdb
    con = duckdb.connect(str(work / "graph.duckdb"))
    con.sql(f"SET extension_directory = '{work / 'ext'}'")
    if pgq:
        con.sql("LOAD duckpgq")
        con.sql("SET active_grammar_extensions = ['duckpgq']")
    return con


def load(graph, work, engine):
    work.mkdir(parents=True, exist_ok=True)
    t = time.perf_counter()
    if engine in ("duckdb", "duckpgq"):
        import duckdb
        con = duckdb.connect(str(work / "graph.duckdb"))
        con.sql(f"SET extension_directory = '{work / 'ext'}'")
        con.sql(f"CREATE TABLE node AS SELECT * FROM '{graph}/nodes.parquet'")
        con.sql(f"CREATE TABLE edges AS SELECT * FROM '{graph}/edges.parquet'")
        if engine == "duckpgq":
            con.sql("INSTALL duckpgq FROM community")
            con.sql("LOAD duckpgq")
            con.sql("SET active_grammar_extensions = ['duckpgq']")
            for name, where in [("runs", "rel = 'runs'"), ("part_of", "rel = 'part_of'"),
                                ("cites", "rel = 'cites'"), ("writes", "rel = 'writes'"),
                                ("code", f"rel IN {CODE}")]:
                con.sql(f"CREATE TABLE {name} AS SELECT * FROM edges WHERE {where}")
            con.sql("""CREATE PROPERTY GRAPH gp VERTEX TABLES (node)
              EDGE TABLES (
                runs SOURCE KEY (src) REFERENCES node (id) DESTINATION KEY (dst) REFERENCES node (id),
                part_of SOURCE KEY (src) REFERENCES node (id) DESTINATION KEY (dst) REFERENCES node (id),
                cites SOURCE KEY (src) REFERENCES node (id) DESTINATION KEY (dst) REFERENCES node (id),
                writes SOURCE KEY (src) REFERENCES node (id) DESTINATION KEY (dst) REFERENCES node (id),
                code SOURCE KEY (src) REFERENCES node (id) DESTINATION KEY (dst) REFERENCES node (id))""")
        con.close()
    else:
        mod = __import__("kuzu" if engine == "kuzu" else "ladybug")
        db = mod.Database(str(work / "graph.db"))
        con = mod.Connection(db)
        con.execute("CREATE NODE TABLE Node(id STRING, kind STRING, name STRING, attrs STRING, PRIMARY KEY (id))")
        con.execute("CREATE REL TABLE E(FROM Node TO Node, rel STRING, via STRING)")
        con.execute(f"COPY Node FROM '{graph}/nodes.parquet'")
        con.execute(f"COPY E FROM '{graph}/edges_ft.parquet'")
    print(json.dumps({"engine": engine, "load_ms": ms(time.perf_counter() - t)}))


def run(work, engine, q):
    import os
    suffix = os.environ.get("GP_GRAPH_SUFFIX", "")

    def fix(text):
        # The scaled copy (bench section of graphs.md) names copy 0's ids with a suffix.
        for lit in ("routine:MoveNormalEnemy", "routine:GameCoreRoutine", "entity:goomba"):
            text = text.replace(f"'{lit}'", f"'{lit}{suffix}'")
        return text
    if engine in ("duckdb", "duckpgq"):
        con = duck(work, engine == "duckpgq")
        text = fix((PGQ if engine == "duckpgq" else SQL)[q])
        go = lambda: con.sql(text).fetchall()
    else:
        mod = __import__("kuzu" if engine == "kuzu" else "ladybug")
        con = mod.Connection(mod.Database(str(work / "graph.db"), read_only=True))
        text = fix(CYPHER[q])

        def go():
            res = con.execute(text)
            out = []
            while res.has_next():
                out.append(tuple(res.get_next()))
            return out
    t = time.perf_counter()
    rows = go()
    first = time.perf_counter() - t
    warm, same_order = [], True
    for _ in range(10):
        t = time.perf_counter()
        again = go()
        warm.append(time.perf_counter() - t)
        # Cypher's UNION gives no order, so runs are compared as sets.
        assert sorted(again) == sorted(rows)
        same_order = same_order and again == rows
    print(json.dumps({"engine": engine, "q": q, "rows": len(rows), "first_ms": ms(first),
                      "warm_median_ms": ms(statistics.median(warm)),
                      "warm_min_ms": ms(min(warm)), "warm_max_ms": ms(max(warm)), "same_order": same_order,
                      "result": [list(map(str, r)) for r in rows]}))


if __name__ == "__main__":
    graph, work, engine = Path(sys.argv[1]), Path(sys.argv[2]), sys.argv[3]
    if len(sys.argv) > 4:
        run(work / engine, engine, sys.argv[4])
    else:
        load(graph, work / engine, engine)
