import re
from timeit import timeit

pattern = r"^(?P<range_desc>(?P<range_upper_desc>(?P<range_upper_desc_prefix><=?)?(?P<range_upper_desc_version>[0-9a-zA-Z_]+(?:[.-][0-9a-zA-Z_]+)*)(\+?)?)(,(?P<range_lower_desc>(?P<range_lower_desc_prefix><|<=|>=?)?(?P<range_lower_desc_version>[0-9a-zA-Z_]+(?:[.-][0-9a-zA-Z_]+)*)(\+?)?))?)$"

test_string = "<=2.0.0,1.0.0+"
match = re.match(pattern, test_string, re.VERBOSE)

if match:
    print("Match found!")
    print(match.groupdict())
else:
    print("No match found.")

from rez.version._version import _VersionRangeParser, AlphanumericVersionToken

samples: int = 100
parser = _VersionRangeParser("1.2.3", make_token=AlphanumericVersionToken)
print(parser._groups)
t = timeit(
    lambda: _VersionRangeParser("1.2.3", make_token=AlphanumericVersionToken),
    number=samples,
)
print(t / samples * 1000000)

a = AlphanumericVersionToken("1.2.3")
