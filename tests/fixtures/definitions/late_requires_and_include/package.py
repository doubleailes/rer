name = 'pkg'
version = '1.0.0'

@late()
@include("foo", "bar")
def requires():
    import foo
    return ["x"]
