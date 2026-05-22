with open('src/doc-format.js', 'r') as f:
    content = f.read()

# Find the loop start and replace the corrupted section
start_marker = '  for (const line of header.slice(1)) {\n'
end_marker = '\n    payload.body = text.slice(separatorIndex + marker.length);\n    return payload;\n  }\n'

start_idx = content.find(start_marker)
end_idx = content.find(end_marker, start_idx)

if start_idx == -1 or end_idx == -1:
    print(f"Could not find markers. start={start_idx}, end={end_idx}")
    # Print context
    idx = content.find('for (const line of header.slice(1))')
    print(repr(content[idx:idx+500]))
else:
    # Replace everything from start_marker to end_marker with the correct code
    correct_loop = '''  for (const line of header.slice(1)) {
      const trimmed = line.trim();

      if (!trimmed) {
        continue;
      }

      const separator = trimmed.indexOf(':');

      if (separator === -1) {
        continue;
      }

      const key = trimmed.slice(0, separator).trim();
      const value = trimmed.slice(separator + 1).trim();

      if (key === 'title') {
        payload.title = value;
        continue;
      }

      if (key === 'summary') {
        payload.summary = value;
        continue;
      }

      if (key === 'tags') {
        payload.tags = value.split(',').map((tag) => tag.trim()).filter(Boolean);
        continue;
      }

      if (key.startsWith('meta.')) {
        const metaKey = key.slice(5).trim();

        if (metaKey) {
          payload.meta[metaKey] = parseValue(value);
        }
      }
    }'''

    new_content = content[:start_idx] + correct_loop + end_marker + content[end_idx + len(end_marker):]
    with open('src/doc-format.js', 'w') as f:
        f.write(new_content)
    print("Fixed parseDocsrcHeader!")
    print(f"Replaced chars from {start_idx} to {end_idx}")
  with open('src/doc-format.js', 'r') as f:
      content = f.read()

  # Find the loop start - use the actual indentation from the file (2 spaces)
  start_marker = '  for (const line of header.slice(1)) {\n'
  # Find the payload.body line which comes after the closing brace of the loop
  end_marker = 'payload.body = text.slice(separatorIndex + marker.length);'

  start_idx = content.find(start_marker)
  end_idx = content.find(end_marker, start_idx)

  if start_idx == -1 or end_idx == -1:
      print(f"Could not find markers. start={start_idx}, end={end_idx}")
      idx = content.find('parseDocsrcHeader')
      print(repr(content[idx:idx+200]))
  else:
      # The correct loop body (using 2-space indentation for the function, 4-space for loop body)
      correct_loop = '''  for (const line of header.slice(1)) {
      const trimmed = line.trim();

      if (!trimmed) {
        continue;
      }

with open('src/doc-format.js', 'r') as f:
    content = f.read()

# Find the loop start - use the actual indentation from the file (2 spaces)
start_marker = '  for (const line of header.slice(1)) {\n'
# Find the payload.body line which comes after the closing brace of the loop
end_marker = 'payload.body = text.slice(separatorIndex + marker.length);'

start_idx = content.find(start_marker)
end_idx = content.find(end_marker, start_idx)

if start_idx == -1 or end_idx == -1:
    print(f"Could not find markers. start={start_idx}, end={end_idx}")
    idx = content.find('parseDocsrcHeader')
    print(repr(content[idx:idx+200]))
else:
    # The correct loop body (using 2-space indentation for the function, 4-space for loop body)
    correct_loop = '''  for (const line of header.slice(1)) {
    const trimmed = line.trim();

    if (!trimmed) {
      continue;
    }

    const separator = trimmed.indexOf(':');

    if (separator === -1) {
      continue;
    }

    const key = trimmed.slice(0, separator).trim();
    const value = trimmed.slice(separator + 1).trim();

    if (key === 'title') {
      payload.title = value;
      continue;
    }

    if (key === 'summary') {
      payload.summary = value;
      continue;
    }

    if (key === 'tags') {
      payload.tags = value.split(',').map((tag) => tag.trim()).filter(Boolean);
      continue;
    }

    if (key.startsWith('meta.')) {
      const metaKey = key.slice(5).trim();

      if (metaKey) {
        payload.meta[metaKey] = parseValue(value);
      }
    }
  }

  '''

    new_content = content[:start_idx] + correct_loop + content[end_idx:]
    with open('src/doc-format.js', 'w') as f:
        f.write(new_content)
    print("Fixed!")
