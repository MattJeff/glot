# glot-bench — 2026-10-18-real

**Broken** = written to the catalog and failing glot's validator (ICU syntax, placeholders, CLDR plural categories, keywords, tags, glossary, pixel width) or the independent FormatJS + `Intl.PluralRules` judge. **Rejected** = refused by glot after retries/escalation and left untranslated instead of shipped broken (raw prompts never reject).

## Summary (all locales)

| Model | Condition | Messages | Broken | Broken (independent judge) | Rejected | Cost |
|---|---|---:|---:|---:|---:|---:|
| claude-haiku-5-5 | glot | 320 | 0.0 % | 0.0 % | 1.9 % | $0.0847 |
| claude-haiku-5-5 | raw | 320 | 14.1 % | 11.9 % | 0.0 % | $0.0212 |
| claude-sonnet-5-5 | glot | 320 | 0.0 % | 0.0 % | 3.1 % | $0.5081 |
| claude-sonnet-5-5 | raw | 320 | 8.8 % | 7.5 % | 0.0 % | $0.2953 |

![broken messages per model](chart.svg)

## Per locale

| Model | Locale | Condition | Broken | Rejected | Cost | Tokens in/out (cache read) | Calls | Latency/call |
|---|---|---|---:|---:|---:|---|---:|---:|
| claude-haiku-5-5 | ar | glot | 0.0 % | 0.0 % | $0.0078 | 8781/13683 (1152) | 3 | 19430 ms |
| claude-haiku-5-5 | ar | raw | 40.0 % | 0.0 % | $0.0026 | 4489/4246 (0) | 1 | 14488 ms |
| claude-haiku-5-5 | de | glot | 0.0 % | 0.0 % | $0.0039 | 5892/6417 (0) | 1 | 23674 ms |
| claude-haiku-5-5 | de | raw | 2.5 % | 0.0 % | $0.0026 | 4486/4160 (0) | 1 | 14111 ms |
| claude-haiku-5-5 | es | glot | 0.0 % | 5.0 % | $0.0164 | 11371/8036 (0) | 4 | 9604 ms |
| claude-haiku-5-5 | es | raw | 0.0 % | 0.0 % | $0.0024 | 4482/3763 (0) | 1 | 12662 ms |
| claude-haiku-5-5 | fr | glot | 0.0 % | 5.0 % | $0.0167 | 11712/9138 (0) | 4 | 10592 ms |
| claude-haiku-5-5 | fr | raw | 5.0 % | 0.0 % | $0.0024 | 4483/3826 (0) | 1 | 19435 ms |
| claude-haiku-5-5 | ja | glot | 0.0 % | 5.0 % | $0.0101 | 11029/7776 (0) | 4 | 9313 ms |
| claude-haiku-5-5 | ja | raw | 5.0 % | 0.0 % | $0.0025 | 4489/4073 (0) | 1 | 15098 ms |
| claude-haiku-5-5 | pl | glot | 0.0 % | 0.0 % | $0.0060 | 7792/10367 (0) | 2 | 21081 ms |
| claude-haiku-5-5 | pl | raw | 42.5 % | 0.0 % | $0.0026 | 4484/4237 (0) | 1 | 14569 ms |
| claude-haiku-5-5 | ru | glot | 0.0 % | 0.0 % | $0.0067 | 8286/11653 (0) | 2 | 25348 ms |
| claude-haiku-5-5 | ru | raw | 17.5 % | 0.0 % | $0.0038 | 4481/6563 (0) | 1 | 23724 ms |
| claude-haiku-5-5 | zh | glot | 0.0 % | 0.0 % | $0.0171 | 10947/6298 (0) | 4 | 7747 ms |
| claude-haiku-5-5 | zh | raw | 0.0 % | 0.0 % | $0.0023 | 4479/3713 (0) | 1 | 13502 ms |
| claude-sonnet-5-5 | ar | glot | 0.0 % | 5.0 % | $0.1087 | 9947/8651 (0) | 3 | 19337 ms |
| claude-sonnet-5-5 | ar | raw | 30.0 % | 0.0 % | $0.0630 | 4489/5328 (0) | 1 | 29222 ms |
| claude-sonnet-5-5 | de | glot | 0.0 % | 0.0 % | $0.0342 | 7318/1788 (0) | 2 | 5911 ms |
| claude-sonnet-5-5 | de | raw | 0.0 % | 0.0 % | $0.0271 | 4486/1731 (0) | 1 | 9632 ms |
| claude-sonnet-5-5 | es | glot | 0.0 % | 0.0 % | $0.0447 | 7733/2758 (0) | 2 | 9633 ms |
| claude-sonnet-5-5 | es | raw | 0.0 % | 0.0 % | $0.0250 | 4482/1529 (0) | 1 | 8894 ms |
| claude-sonnet-5-5 | fr | glot | 0.0 % | 5.0 % | $0.0739 | 10157/5131 (0) | 3 | 12248 ms |
| claude-sonnet-5-5 | fr | raw | 5.0 % | 0.0 % | $0.0284 | 4483/1862 (0) | 1 | 9811 ms |
| claude-sonnet-5-5 | ja | glot | 0.0 % | 5.0 % | $0.0371 | 9316/1616 (0) | 3 | 3861 ms |
| claude-sonnet-5-5 | ja | raw | 5.0 % | 0.0 % | $0.0235 | 4489/1371 (0) | 1 | 8423 ms |
| claude-sonnet-5-5 | pl | glot | 0.0 % | 5.0 % | $0.0886 | 9972/6636 (0) | 3 | 15904 ms |
| claude-sonnet-5-5 | pl | raw | 12.5 % | 0.0 % | $0.0553 | 4484/4559 (0) | 1 | 25396 ms |
| claude-sonnet-5-5 | ru | glot | 0.0 % | 5.0 % | $0.0805 | 10443/5734 (0) | 3 | 13912 ms |
| claude-sonnet-5-5 | ru | raw | 12.5 % | 0.0 % | $0.0506 | 4481/4088 (0) | 1 | 23757 ms |
| claude-sonnet-5-5 | zh | glot | 0.0 % | 0.0 % | $0.0403 | 7573/2350 (0) | 2 | 8804 ms |
| claude-sonnet-5-5 | zh | raw | 5.0 % | 0.0 % | $0.0223 | 4479/1256 (0) | 1 | 7618 ms |

## Why messages broke

| Model | Condition | Issues (count) | Broken per category |
|---|---|---|---|
| claude-haiku-5-5 | glot | — | — |
| claude-haiku-5-5 | raw | independent:plural_category_missing 32, independent:tag_mismatch 6, plural_category_missing 32, tag_mismatch 6, too_wide 7 | nested 10, placeholders 4, plural 16, plural_offset 2, tags 6, width 7 |
| claude-sonnet-5-5 | glot | — | — |
| claude-sonnet-5-5 | raw | independent:plural_category_missing 12, independent:tag_mismatch 14, plural_category_missing 12, tag_mismatch 14, too_wide 4 | nested 13, placeholders 2, plural 4, plural_offset 1, tags 4, width 4 |
