name = 'cmds'
version = '1.0'
timestamp = 1
requires = ['dep']
def commands():
    env.PATH.append('{root}/bin')
