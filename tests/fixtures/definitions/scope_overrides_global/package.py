name = 'pkg'
version = '1.0.0'

config = "global"
with scope("config") as c:
    c.a = 1
