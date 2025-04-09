import nox


@nox.session(python=['3.13'])
def ci(session):
    """Build and run tests."""
    session.install('maturin', 'pytest')
    session.run('maturin', 'develop')
    session.run('pytest', '-v')
