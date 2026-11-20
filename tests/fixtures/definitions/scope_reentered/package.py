name = 'pkg'
version = '1.0.0'

with scope("config") as c:
    c.a = 1
with scope("config") as c:
    c.b = 2
    c.a = 3
