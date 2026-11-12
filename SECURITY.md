# Security policy

## Supported versions

Only the latest release receives security fixes. Openify updates itself in the
background, so please update to the newest version before reporting.

| Version | Supported |
|---|---|
| Latest release | Yes |
| Older releases | No |

## Reporting a vulnerability

Please do **not** open a public issue for a security problem.

Use GitHub's private reporting instead: open the
[Security tab](https://github.com/FurkanKY/Openify/security) of the repository
and choose **Report a vulnerability**. This creates a private advisory that only
you and the maintainer can see.

Please include:

- what the problem is and what an attacker could do with it,
- the Openify version and Windows version,
- steps to reproduce it, or a short proof of concept.

Do not include your Spotify credentials or tokens, and remove them from any log
you attach.

You can expect an acknowledgement within a few days. Openify is a small
open-source project run by one person, so fixes are made on a best-effort basis;
the report will be kept private until a fixed release is out.

## What is in scope

Problems in Openify's own code and packaging: for example how it stores the
Spotify session, handles the sign-in redirect on `127.0.0.1`, applies updates, or
reads files and network responses.

Problems in Spotify's own services, or in the upstream projects Openify builds on
(librespot, gpui and the other crates it uses), should be reported to those
projects. If a vulnerability in a dependency affects Openify, a report here is
still welcome.
