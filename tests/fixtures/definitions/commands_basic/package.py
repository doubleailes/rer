name = 'pkg'
version = '1.0.0'

def commands():
    env.PATH.append("{root}/bin")
    env.FOO = "bar"
    if building:
        env.BUILDING = 1
