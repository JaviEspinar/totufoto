# Security

## What Totufoto protects against, and what it doesn't

Totufoto is a gallery for your own computer and your own network. It has **no login**, by design: whoever can reach the server can use all of it.

What the server can do, for anyone who can reach it:

- show every photo in the gallery and download the original files;
- rotate photos (the files are rewritten) and delete them (moved to the bin, or deleted for good when the drive has no bin);
- add and remove photo folders, and list the folders of the computer to choose one.

So:

- **By default it only listens on this computer** (`127.0.0.1`). The desktop app always works that way.
- **With `--host 0.0.0.0`** every device on your network can do all of the above. Only use it on a network where you trust every device and person, and never make the server reachable from the internet (no port forwarding, no public reverse proxy).

What it does protect against, on every address:

- **Web pages on other sites** using the gallery through your browser. Requests for a host name other than an IP address, `localhost`, the computer's own name or a name given with `--allow-host` are refused, which blocks DNS rebinding, and requests that change something are refused when the browser says they come from another site.
- **Paths outside the gallery**: files are only served, rotated or deleted when they are photos indexed from your folders.

## Reporting a vulnerability

Please report security problems privately through GitHub: open the repository's **Security** tab and choose **Report a vulnerability**. Don't open a public issue for them.

Say which version you use, how the server is started (command-line options, or the desktop app) and how to reproduce the problem. You should get an answer within a week.

## Supported versions

Fixes go into the latest release only. Update to it before reporting.
