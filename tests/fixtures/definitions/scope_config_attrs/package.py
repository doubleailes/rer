name = 'pkg'
version = '1.0.0'

with scope("config") as config:
    config.release_packages_path = "/rel"
    config.build_thread_count = "logical_cores"
    config.plugins.release_hook.emailer.recipients = ["a@b"]
    config.plugins.release_hook.emailer.sender = "x"
