---
title: Front matter, the YAML kind
author: Scribobulate test fixture
tags: [metadata, disclosure, code-block]
draft: false
tail: TAILMARKER
---

# Front matter

The block above this heading is YAML front matter. In the preview it renders as a
collapsed disclosure labelled *Frontmatter*; expanding it shows the metadata as a YAML
code block, with a copy button of its own.

Nothing about it should reach the outline sidebar, the word count, or an export.

## A heading below it

The outline must list exactly two headings for this document — *Front matter* and *A
heading below it* — and never a third one whose text is the metadata.

<details>
<summary>An authored disclosure, for comparison</summary>

This one is written by hand, and it must behave identically to the synthetic one above
it: its own arrow, its own fold, its own body.

</details>

Prose after the authored block.
