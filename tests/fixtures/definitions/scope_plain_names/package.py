name = 'pkg'
version = '1.0.0'

x = 1
with scope("config"):
    x = 1
    y = 2
    z = {"a": {"b": 1}}
requires = ["r"]
