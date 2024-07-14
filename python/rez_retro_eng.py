from rez.version._version import (
    _VersionRangeParser,
    AlphanumericVersionToken,
    _SubToken,
    Version,
)
from rez.version._requirement import Requirement

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

a = Requirement("cffi-1.8+<1.11.3|1.11.3.1+")
b = _VersionRangeParser("1.13.0|2.1.0", make_token=AlphanumericVersionToken)
print(b._groups)
a = Requirement("~understatement==x86_64")
print(a.range)
b = _VersionRangeParser("==x86_64", make_token=AlphanumericVersionToken)
#print(b._groups)
a = Requirement("!understatement==x86_64")
print(a.range)