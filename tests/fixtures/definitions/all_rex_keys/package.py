name = 'pkg'
version = '1.0.0'

def pre_commands():
    env.PRE = 1
def commands():
    env.CMD = 1
def post_commands():
    env.POST = 1
def pre_build_commands():
    env.PB = 1
def pre_test_commands():
    env.PT = 1
