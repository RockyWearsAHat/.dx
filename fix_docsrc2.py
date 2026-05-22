with open('src/doc-format.js', 'r') as f:
    content = f.read()

# Find start of the corrupted section (the loop)
start_marker = '  for (const line of header.slice(1)) {\n'
# Find the payload.body line which comes after the loop's closing brace
end_marker = 'payload.body = text.slice(separatorIndex + marker.length);'

start_idx = content.find(start_marker)
end_idx = content.find(end_marker, start_idx)

if start_idx == -1 or end_idx == -1:
    print(f"Could not find markers. start={start_idx}, end={end_idx}")
    idx = content.find('parseDocsrcHeader')
    print(repr(content[idx:idx+300]))
else:
    # The correct loop body - use indentation matching the surrounding function (2 spaces per level)
    correct_loop = (
        '  for (const line of header.slice(1)) {\n'
        '    const trimmed = line.trim();\n'
        '\n'
        '    if (!trimmed) {\n'
        '      continue;\n'
        '    }\n'
        '\n'
        "    const separator = trimmed.indexOf(':');\n"
        '\n'
        '    if (separator === -1) {\n'
        '      continue;\n'
        '    }\n'
        '\n'
        '    const key = trimmed.slice(0, separator).trim();\n'
        '    const value = trimmed.slice(separator + 1).trim();\n'
        '\n'
        "    if (key === 'title') {\n"
        '      payload.title = value;\n'
        '      continue;\n'
        '    }\n'
        '\n'
        "    if (key === 'summary') {\n"
        '      payload.summary = value;\n'
        '      continue;\n'
        '    }\n'
        '\n'
        "    if (key === 'tags') {\n"
        "      payload.tags = value.split(',').map((tag) => tag.trim()).filter(Boolean);\n"
        '      continue;\n'
        '    }\n'
        '\n'
        "    if (key.startsWith('meta.')) {\n"
        '      const metaKey = key.slice(5).trim();\n'
        '\n'
        '      if (metaKey) {\n'
        '        payload.meta[metaKey] = parseValue(value);\n'
        '      }\n'
        '    }\n'
        '  }\n'
        '\n'
        '  '
    )

    new_content = content[:start_idx] + correct_loop + content[end_idx:]
    with open('src/doc-format.js', 'w') as f:
        f.write(new_content)
    print("Fixed!")
    print(f"Replaced from char {start_idx} to {end_idx}")
