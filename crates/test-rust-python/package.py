# -*- coding: utf-8 -*-

name = 'baker'

version = '2.0.0'

description = 'UNKNOWN'

requires = [
    'qt_utils-1',
    'appy-1',
    'voodoo-1',
    'parentswitcher-1'
]

variants = [
    ['python-3.7'],
    ['python-3.9']
]

def commands():
    env.PYTHONPATH.append('{root}/python')

timestamp = 1712669699

hashed_variants = True

is_pure_python = True

pip_name = 'baker (2.0.0)'

from_pip = True

format_version = 2

# Added by lorenzo (a lot of bullshit from copilot)

authors = ["lorenzo", "gabriele", "giacomo"]

build_requires = [
    'python-3',
    'pip-19.2',
    'setuptools-44.1',
    'wheel-0.33',
    'baker-2.0.0'
]

plugin_for = "maya"