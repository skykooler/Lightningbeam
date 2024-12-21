const darkMode = window.matchMedia && window.matchMedia('(prefers-color-scheme: dark)').matches;
const backgroundColor = darkMode ? "#333" : "#ccc"
const foregroundColor = darkMode ? "#888" : "#ddd"
const highlight = darkMode ? "#4f4f4f" : "#ddd"
const shadow = darkMode ? "#111" : "#999"
const shade = darkMode ? "#222" : "#aaa"
const layerHeight = 50
const iconSize = 25
const layerWidth = 300
const frameWidth = 25
const gutterHeight = 15
const scrubberColor = "#cc2222"
const labelColor = darkMode ? "white" : "black"

export {
    darkMode,
    backgroundColor,
    foregroundColor,
    highlight,
    shadow,
    shade,
    layerHeight,
    iconSize,
    layerWidth,
    frameWidth,
    gutterHeight,
    scrubberColor,
    labelColor
}