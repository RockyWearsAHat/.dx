export default async function* customReporter(source) {
  let passCount = 0;

  for await (const event of source) {
    if (event.type === 'test:pass') {
      passCount++;
    }
  }

  // Output the summary in the expected format
  yield `files ${passCount} pass\n`;
}
