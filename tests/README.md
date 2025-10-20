# Lightningbeam UI Tests

Automated UI tests for Lightningbeam using WebdriverIO and tauri-driver.

## Prerequisites

1. **Install test dependencies**:
   ```bash
   pnpm add -D @wdio/cli @wdio/local-runner @wdio/mocha-framework @wdio/spec-reporter @wdio/globals
   ```

   **Important**: If the `@wdio/local-runner` package hangs during installation, you must install it in your native OS environment (not in a container). The pnpm store can have conflicts when switching between different OS contexts. If you originally ran `pnpm install` on your host system, install the test dependencies there as well.

2. **Build the application** - Tests require the release build:
   ```bash
   pnpm tauri build
   ```

   **Note**: The debug build (`pnpm tauri build --debug`) won't work for tests because it expects a dev server to be running. Tests use the self-contained release build.

3. **Install tauri-driver** - Download from [tauri-apps/tauri releases](https://github.com/tauri-apps/tauri/releases):
   ```bash
   # Linux example
   cargo install tauri-driver
   # Or download binary and add to PATH
   ```

## Running Tests

### 1. Start tauri-driver
In a separate terminal, start tauri-driver:
```bash
tauri-driver --port 4444
```

### 2. Run all tests
```bash
pnpm test
```

### Run tests in watch mode
```bash
pnpm test:watch
```

### Run specific test file
```bash
pnpm wdio run wdio.conf.js --spec tests/specs/shapes.test.js
```

## Test Structure

```
tests/
├── helpers/
│   ├── app.js          # App lifecycle helpers
│   ├── canvas.js       # Canvas interaction utilities
│   └── assertions.js   # Custom assertions
├── specs/
│   ├── shapes.test.js       # Shape drawing tests
│   ├── grouping.test.js     # Shape grouping tests
│   └── paint-bucket.test.js # Paint bucket tool tests
└── fixtures/           # Test data files
```

## Writing Tests

### Example: Drawing a Rectangle

```javascript
import { drawRectangle } from '../helpers/canvas.js';
import { assertShapeExists } from '../helpers/assertions.js';

it('should draw a rectangle', async () => {
  await drawRectangle(100, 100, 200, 150);
  await assertShapeExists(200, 175, 'Rectangle should exist at center');
});
```

### Available Helpers

#### Canvas Helpers (`canvas.js`)
- `clickCanvas(x, y)` - Click at coordinates
- `dragCanvas(fromX, fromY, toX, toY)` - Drag operation
- `drawRectangle(x, y, width, height)` - Draw rectangle
- `drawEllipse(x, y, width, height)` - Draw ellipse
- `selectTool(toolName)` - Select a tool by name
- `selectMultipleShapes(points)` - Select multiple shapes with Ctrl
- `useKeyboardShortcut(key, withCtrl)` - Use keyboard shortcuts
- `getPixelColor(x, y)` - Get color at coordinates
- `hasShapeAt(x, y)` - Check if shape exists at point

#### Assertion Helpers (`assertions.js`)
- `assertShapeExists(x, y, message)` - Assert shape at coordinates
- `assertNoShapeAt(x, y, message)` - Assert no shape at coordinates
- `assertPixelColor(x, y, color, message)` - Assert pixel color
- `assertColorApproximately(color1, color2, tolerance)` - Fuzzy color match

## Adding Data Attributes for Testing

To make UI elements easier to test, add `data-tool` attributes to tool buttons in the UI:

```javascript
// Example in main.js
const rectangleTool = document.createElement('button');
rectangleTool.setAttribute('data-tool', 'rectangle');
```

Current expected data attributes:
- `data-tool="rectangle"` - Rectangle tool button
- `data-tool="ellipse"` - Ellipse tool button
- `data-tool="dropper"` - Paint bucket/dropper tool button
- Add more as needed...

## Platform Support

- **Linux**: Full support with webkit2gtk
- **Windows**: Full support with WebView2
- **macOS**: Limited support (no WKWebView driver available)

## Troubleshooting

### Tests fail to start
- Ensure the release build exists: `./src-tauri/target/release/lightningbeam`
- Check that `tauri-driver` is in your PATH

### Canvas interactions don't work
- Verify that tool buttons have `data-tool` attributes
- Check that canvas element is present with `document.querySelector('canvas')`

### Screenshots directory missing
```bash
mkdir -p tests/screenshots
```

## CI Integration

See `.github/workflows/` for example GitHub Actions configuration (to be added).

## Future Enhancements

- [ ] Add color picker test helpers
- [ ] Add timeline/keyframe test helpers
- [ ] Add layer management test helpers
- [ ] Visual regression testing with screenshot comparison
- [ ] Performance benchmarks
- [ ] Add Tauri commands for better state inspection
