# Compatibility strategy

crusty-dlp delegates extractor maintenance to yt-dlp instead of duplicating its
large, fast-changing site catalog. The application supplies safe, reusable
compatibility mechanisms for recurring failure classes.

| Failure class / examples | Supported approach |
| --- | --- |
| TLS fingerprinting, some anti-bot frontends | Select a target in the Impersonation panel. |
| BoyfriendTV reports `Unsupported URL` | The bundled `BoyfriendTVIE` plugin parses its public source list; crusty-dlp also enables impersonation automatically. |
| PMVHaven reports `Unsupported URL` | The bundled `PMVHavenIE` plugin reads its public VideoObject metadata and HLS manifest. |
| SpankBang returns HTTP 403 | The bundled extractor parses public `stream_data` media variants. Open SpankBang in a local browser, press `b` to select that browser, and retry so the initial page request can reuse fresh cookies. |
| A site works only in an already-authorized browser session; some YouTube and Rule34Video failures | Press `b` and select the local browser. Cookies are read directly by yt-dlp and never stored by crusty-dlp. |
| YouTube JavaScript challenge warnings or missing formats | Install Deno (`sudo pacman -S deno` on Arch/CachyOS). The Arch yt-dlp package already depends on `yt-dlp-ejs`. |
| Outdated or broken extractor | Update yt-dlp. Site extractor fixes belong upstream and should not be hard-coded into the TUI. |
| DRM, CAPTCHA, paywall, or inaccessible content | Not supported. crusty-dlp does not bypass access controls. |

These mechanisms do not guarantee that every website will work. Websites and
extractors change continuously. A site-specific workaround is included only
when it uses public media URLs, has a maintainable implementation, and does not
bypass an access-control boundary.

## Research snapshot

This policy was reviewed against yt-dlp's open issue tracker and known-issues
documentation on 2026-07-06. Relevant recurring reports involved YouTube's
external JavaScript challenges, browser-session checks, HTTP 403 responses, and
TLS impersonation. Re-check upstream before adding a permanent site rule.
