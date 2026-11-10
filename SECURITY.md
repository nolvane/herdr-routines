# Security

Please report a vulnerability privately through GitHub:
**Security → Report a vulnerability** on this repository. Do not open a public
issue for it.

The plugin never sees a token or a login: it starts the coding agents you
choose through Herdr, in your own shell, with your own logins. The most likely
issues are in how it handles what it reads back (routine files, agent output
shown in the terminal) and in the commands it builds to start an agent
(folder, agent arguments, prompt).
