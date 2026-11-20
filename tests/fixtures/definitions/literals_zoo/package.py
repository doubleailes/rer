name = 'pkg'
version = '1.0.0'

a = [1, -2, +3, 0x1F, 0o17, 0b101, 1_000_000, 0]
b = (1.5, -0.25, 1e10, 2.5E-3, .5, 5.)
c = {"k": None, "k2": [1, (2, 3)]}
d = ("single",)
e = ()
f = ((1))
g = {}
h = [[], [[]], {"n": {"m": []}}]
i = """triple
quoted"""
j = 'implicit' "concat" """ation"""
k = u"unicode prefix"
l = R"raw \n %s"
m = "esc: \' \" \\ \a \b \f \n \r \t \v \0 \7 \101 \x41 \u00e9 \U0001F600 \q"
n = "line\
continued"
o = [
    "trailing",  # comment
    "comma",
]
p = 'é ü 漢字'
q = -0.0
r = 9223372036854775807
s = {"__dunder": 1, "_single": 2, "x": {"__deep": 3}}
