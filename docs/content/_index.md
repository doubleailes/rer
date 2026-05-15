+++
title = "rer — Rez En Rust"


# The homepage contents
[extra]
lead = "A faithful Rust port of <a href=\"https://github.com/AcademySoftwareFoundation/rez\">rez</a>'s package solver — callable from Python via PyO3, resolves match rez 1:1."
url = "/docs/getting-started/introduction/"
url_button = "Get started"
repo_version = "GitHub v0.1.0-rc.4"
repo_license = "MIT-licensed."
repo_url = "https://github.com/doubleailes/rer"

# Menu items
[[extra.menu.main]]
name = "Docs"
section = "docs"
url = "/docs/getting-started/introduction/"
weight = 10

[[extra.list]]
title = "Rez-faithful"
content = "A port of <code>rez/src/rez/solver.py</code> — weak (<code>~</code>) and conflict (<code>!</code>) requirements, variant selection order, extract / intersect / reduce / split, and implicit backtracking. Output matches rez 1:1."

[[extra.list]]
title = "1:1 on the rez benchmark"
content = "Every one of the 188 cases in rez's bundled benchmark dataset resolves to rez's own recorded result — same status, same package set."

[[extra.list]]
title = "Python-callable"
content = "<code>pip install pyrer</code>, <code>import pyrer</code>. The PyO3 bridge runs the ported solver against an in-memory package repository handed in from rez."

[[extra.list]]
title = "Fast"
content = "On the rez benchmark, on one machine: ~44s for all 188 resolves versus ~206s for <code>rez benchmark</code> on rez 3.3.0. Same-machine context, not a lab claim."

[[extra.list]]
title = "In-memory by design"
content = "rer never reads the filesystem. The host (rez) loads packages and passes them in as JSON; rer solves and hands the resolution back."

[[extra.list]]
title = "MIT licensed"
content = "Open source. Built to slot into existing rez workflows without disturbing them."

+++
