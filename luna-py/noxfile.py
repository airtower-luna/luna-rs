import nox


@nox.session(python=['3.13'])
def ci(session):
    """Build and run tests."""
    session.install('.', 'pytest')
    session.run('pytest', '-v')
