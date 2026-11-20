name = 'pkg'
version = '1.0.0'

@include("utils")
def commands():
    utils.setup(env)
