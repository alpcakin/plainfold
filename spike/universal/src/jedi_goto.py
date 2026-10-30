# Second opinion for Python: Jedi (an independent engine) answers "go to definition" for the same call sites.
# Usage: <python of the environment> jedi_goto.py <project root> <sites.json> <out.json>
import json
import sys

import jedi

root, sites_path, out_path = sys.argv[1:4]
sites = json.load(open(sites_path))
project = jedi.Project(root)
scripts = {}
out = []
for s in sites:
    path = s["abs"]
    if path not in scripts:
        scripts[path] = jedi.Script(path=path, project=project)
    res = None
    try:
        names = scripts[path].goto(s["at"]["line"] + 1, s["at"]["col"], follow_imports=True)
        for n in names:
            if n.module_path is not None and n.line is not None:
                res = {"file": str(n.module_path), "line": n.line - 1}
                break
    except Exception as e:  # a crash on one site must not hide the others
        res = {"error": type(e).__name__}
    out.append(res)
json.dump(out, open(out_path, "w"))
