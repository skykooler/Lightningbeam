let overlay;
let newFileDialog;

function createNewFileDialog(callback) {
    overlay = document.createElement('div');
    overlay.id = 'overlay';
    document.body.appendChild(overlay);

    newFileDialog = document.createElement('div');
    newFileDialog.id = 'newFileDialog';
    newFileDialog.classList.add('hidden');
    document.body.appendChild(newFileDialog);

    // Create dialog content dynamically
    const title = document.createElement('h3');
    title.textContent = 'Create New File';
    newFileDialog.appendChild(title);

    // Create Width input
    const widthLabel = document.createElement('label');
    widthLabel.setAttribute('for', 'width');
    widthLabel.classList.add('dialog-label');
    widthLabel.textContent = 'Width:';
    newFileDialog.appendChild(widthLabel);

    const widthInput = document.createElement('input');
    widthInput.type = 'number';
    widthInput.id = 'width';
    widthInput.classList.add('dialog-input');
    widthInput.value = '1500'; // Default value
    newFileDialog.appendChild(widthInput);

    // Create Height input
    const heightLabel = document.createElement('label');
    heightLabel.setAttribute('for', 'height');
    heightLabel.classList.add('dialog-label');
    heightLabel.textContent = 'Height:';
    newFileDialog.appendChild(heightLabel);

    const heightInput = document.createElement('input');
    heightInput.type = 'number';
    heightInput.id = 'height';
    heightInput.classList.add('dialog-input');
    heightInput.value = '1000'; // Default value
    newFileDialog.appendChild(heightInput);

    // Create Create button
    const createButton = document.createElement('button');
    createButton.textContent = 'Create';
    createButton.classList.add('dialog-button');
    createButton.onclick = createNewFile;
    newFileDialog.appendChild(createButton);


    // Create the new file (simulation)
    function createNewFile() {
        const width = document.getElementById('width').value;
        const height = document.getElementById('height').value;
        console.log(`New file created with width: ${width} and height: ${height}`);
        console.log(callback)
        callback(width, height)

        // Add any further logic to handle the new file creation here

        closeDialog(); // Close the dialog after file creation
    }

    // Close the dialog if the overlay is clicked
    overlay.onclick = closeDialog;
}

// Show the dialog
function showNewFileDialog() {
    overlay.style.display = 'block';
    newFileDialog.style.display = 'block';
}

// Close the dialog
function closeDialog() {
    overlay.style.display = 'none';
    newFileDialog.style.display = 'none';
}
export { createNewFileDialog, showNewFileDialog, closeDialog };