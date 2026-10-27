import io
import os
import re

root = "crates/symbio-core/src"
ents = (
    "user", "turn", "chat", "task", "memory", "commitment", "verdict",
    "artifact", "authz", "session", "skill", "tool", "agent", "file",
)
pat = re.compile(r'"((?:%s)\.[a-z_]+(?:\.[a-z_]+)*)"' % "|".join(ents))
hits = []
for dp, _, fs in os.walk(root):
    for f in fs:
        if not f.endswith(".rs") or f.endswith(".test.rs"):
            continue
        p = os.path.join(dp, f)
        s = io.open(p, encoding="utf-8").read()
        for m in pat.finditer(s):
            line = s[: m.start()].count("\n") + 1
            hits.append((p.replace(os.sep, "/"), line, m.group(1)))
for h in hits:
    print(h)
print("total", len(hits))
