const { basename, dirname, join } = window.__TAURI__.path;

let overlay;
let newFileDialog;

let displayFiles

function createNewFileDialog(newFileCallback, openFileCallback, config) {
    overlay = document.createElement('div');
    overlay.id = 'overlay';
    document.body.appendChild(overlay);

    newFileDialog = document.createElement('div');
    newFileDialog.id = 'newFileDialog';
    newFileDialog.classList.add('hidden');
    document.body.appendChild(newFileDialog);

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
    console.log(config.fileWidth)
    widthInput.value = config.fileWidth;
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
    heightInput.value = config.fileHeight;
    newFileDialog.appendChild(heightInput);

    // Create FPS input
    const fpsLabel = document.createElement('label');
    fpsLabel.setAttribute('for', 'fps');
    fpsLabel.classList.add('dialog-label');
    fpsLabel.textContent = 'Frames per Second:';
    newFileDialog.appendChild(fpsLabel);

    const fpsInput = document.createElement('input');
    fpsInput.type = 'number';
    fpsInput.id = 'fps';
    fpsInput.classList.add('dialog-input');
    fpsInput.value = config.framerate;
    newFileDialog.appendChild(fpsInput);

    // Create Create button
    const createButton = document.createElement('button');
    createButton.textContent = 'Create';
    createButton.classList.add('dialog-button');
    createButton.onclick = createNewFile;
    newFileDialog.appendChild(createButton);

    // Recent Files Section
    const recentFilesTitle = document.createElement('h4');
    recentFilesTitle.textContent = 'Recent Files';
    newFileDialog.appendChild(recentFilesTitle);

    const recentFilesList = document.createElement('ul');
    recentFilesList.id = 'recentFilesList';
    newFileDialog.appendChild(recentFilesList);

    function createNewFile() {
        const width = parseInt(document.getElementById('width').value);
        const height = parseInt(document.getElementById('height').value);
        const fps = parseInt(document.getElementById('fps').value);
        console.log(`New file created with width: ${width} and height: ${height}`);
        newFileCallback(width, height, fps)
        closeDialog();
    }


    async function displayRecentFiles(recentFiles) {
        const recentFilesList = document.getElementById('recentFilesList');
        const recentFilesTitle = document.querySelector('h4');

        recentFilesList.innerHTML = '';

        // Only show the list if there are recent files
        if (recentFiles.length === 0) {
            recentFilesTitle.style.display = 'none';
        } else {
            recentFilesTitle.style.display = 'block';
            const filenames = {};

            for (let filePath of recentFiles) {
                const filename = await basename(filePath);
                const dirPath = await dirname(filePath);

                if (!filenames[filename]) {
                    filenames[filename] = [];
                }
                filenames[filename].push(dirPath);
            }

            Object.keys(filenames).forEach((filename) => {
                const filePaths = filenames[filename];

                // If only one directory, just display the filename
                if (filePaths.length === 1) {
                    const listItem = document.createElement('li');
                    listItem.textContent = filename;
                    listItem.onclick = () => openFile(filePaths[0], filename);
                    recentFilesList.appendChild(listItem);
                } else {
                    // For duplicates, display each directory with the filename
                    filePaths.forEach((dirPath) => {
                        const listItem = document.createElement('li');
                        listItem.innerHTML = `${filename} (${dirPath}/)`;
                        listItem.onclick = () => openFile(dirPath, filename);
                        recentFilesList.appendChild(listItem);
                    });
                }
            });
        }
    }

    displayFiles = displayRecentFiles

    async function openFile(dirPath, filename) {
        console.log(await join(dirPath, filename))
        openFileCallback(await join(dirPath, filename))
        closeDialog()
    }

    overlay.onclick = closeDialog;
}

function showNewFileDialog(config) {
    overlay.style.display = 'block';
    newFileDialog.style.display = 'block';
    displayFiles(config.recentFiles); // Reload the recent files
}

function closeDialog() {
    overlay.style.display = 'none';
    newFileDialog.style.display = 'none';
}
export { createNewFileDialog, showNewFileDialog, closeDialog };