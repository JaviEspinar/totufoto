# Security

## What Totufoto protects against, and what it doesn't

Totufoto is a gallery for your own computer and your own network. It has **no login**, by design: whoever can reach the server can use all of it.

What the server can do, for anyone who can reach it:

- show every photo in the gallery and download the original files;
- rotate photos (the files are rewritten) and delete them (moved to the bin, or deleted for good when the drive has no bin);
- add and remove photo folders, and list the folders of the computer to choose one. Because any folder can be added, this reaches further than the gallery: someone on your network can add a folder you never shared, then view and delete the images in it.

So:

- **By default it only listens on this computer** (`127.0.0.1`). The desktop app always works that way.
- **With `--host 0.0.0.0`** every device on your network can do all of the above. Only use it on a network where you trust every device and person, and never make the server reachable from the internet (no port forwarding, no public reverse proxy).

What it does protect against, on every address:

- **Web pages on other sites** using the gallery through your browser. Requests for a host name other than an IP address, `localhost`, the computer's own name or a name given with `--allow-host` are refused, which blocks DNS rebinding, and requests that change something are refused when the browser says they come from another site.
- **Paths outside the gallery**: files are only served, rotated or deleted when they are photos indexed from the gallery's folders (which, as above, anyone on the network can add to).

This is deliberate: Totufoto is meant for a home network, and managing its folders from another computer's browser is part of how it is used there. If that's not acceptable for your network, keep the default address (`127.0.0.1`) and use the desktop app or a browser on the same computer.

## Reporting a vulnerability

Please report security problems privately through GitHub: open the repository's **Security** tab and choose **Report a vulnerability**. Don't open a public issue for them.

Say which version you use, how the server is started (command-line options, or the desktop app) and how to reproduce the problem. You should get an answer within a week.

## Supported versions

Fixes go into the latest release only. Update to it before reporting.
