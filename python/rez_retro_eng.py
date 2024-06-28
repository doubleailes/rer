from rez.version._version import (
    _VersionRangeParser,
    AlphanumericVersionToken,
    _SubToken,
    Version,
)

a = Version("1.2.3-alpha+beta")
print(a.tokens, a.seps)
a = Version("2.0.0_")
print(a.tokens, a.seps)
a = Version("2.0.0")
b = a.next()
print(b.tokens, b.seps)
a = AlphanumericVersionToken("1")
print(a.subtokens[0])
b = AlphanumericVersionToken("1_")
print(a.subtokens, b.subtokens)
assert a.next() == b
