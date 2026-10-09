# SVG XInclude refusal (TDD 2.23c)

Every image below except the first uses XInclude (harmlessly: it includes a red
square from a `data:` URI). On Linux and Windows each shows the broken-image
placeholder; on macOS each shows a red square.

Plain SVG, always shown (a green square):

![plain](plain.svg)

Namespace under the prefix `q`:

![prefixed](prefixed.svg)

Namespace spelled with a character reference:

![hidden](hidden.svg)

UTF-16 with a byte-order mark:

![utf16](utf16.svg)

Gzip-compressed `.svgz`:

![compressed](compressed.svgz)

An SVG embedding the prefixed one as a base64 `data:` image:

![nested](nested.svg)
