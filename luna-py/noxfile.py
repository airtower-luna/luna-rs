import nox

nox.options.download_python = 'never'


@nox.session(python=['3.13'])
def ci(session):
    """Build and run tests."""
    session.install('.', 'pytest')
    session.run('pytest', '-v')


@nox.session
def stubs(session):
    """Check if stubs match runtime library."""
    session.install('.', 'mypy')
    session.run('stubtest', '--allowlist', 'stubtest-allow.txt', 'luna')
