name = 'pkg'
version = '1.0.0'

def commands():
    def helper(x):
        return x
    env.A = helper(1)
