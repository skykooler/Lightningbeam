function titleCase(str) {
    return str.charAt(0).toUpperCase() + str.slice(1).toLowerCase();
}

function getMousePositionFraction(event, element) {
    const rect = element.getBoundingClientRect(); // Get the element's position and size
    
    if (element.classList.contains('horizontal-grid')) {
      // If the element has the "horizontal-grid" class, calculate the horizontal position (X)
      const xPos = event.clientX - rect.left; // Mouse X position relative to the element
      const fraction = xPos / rect.width; // Fraction of the width
      return Math.min(Math.max(fraction, 0), 1); // Ensure the fraction is between 0 and 1
    } else if (element.classList.contains('vertical-grid')) {
      // If the element has the "vertical-grid" class, calculate the vertical position (Y)
      const yPos = event.clientY - rect.top; // Mouse Y position relative to the element
      const fraction = yPos / rect.height; // Fraction of the height
      return Math.min(Math.max(fraction, 0), 1); // Ensure the fraction is between 0 and 1
    }
    return 0; // If neither class is present, return 0 (or handle as needed)
  }

export { titleCase, getMousePositionFraction };