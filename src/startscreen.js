const { basename, dirname, join } = window.__TAURI__.path;

let startScreenContainer;
let onProjectStartCallback;

/**
 * Creates the start screen UI
 * @param {Function} callback - Called when user selects a project type or opens a file
 *   callback receives: { type: 'new'|'reopen'|'recent', projectFocus?: string, filePath?: string, width?: number, height?: number, fps?: number }
 */
export function createStartScreen(callback) {
    onProjectStartCallback = callback;

    startScreenContainer = document.createElement('div');
    startScreenContainer.id = 'startScreen';
    startScreenContainer.className = 'start-screen';

    // Create welcome title
    const title = document.createElement('h1');
    title.textContent = 'Welcome to Lightningbeam!';
    title.className = 'start-screen-title';
    startScreenContainer.appendChild(title);

    // Create main content container
    const contentContainer = document.createElement('div');
    contentContainer.className = 'start-screen-content';
    startScreenContainer.appendChild(contentContainer);

    // Left panel - Recent files
    const leftPanel = createLeftPanel();
    contentContainer.appendChild(leftPanel);

    // Right panel - New project
    const rightPanel = createRightPanel();
    contentContainer.appendChild(rightPanel);

    document.body.appendChild(startScreenContainer);
}

function createLeftPanel() {
    const leftPanel = document.createElement('div');
    leftPanel.className = 'start-screen-left-panel';

    // Reopen last session section
    const reopenSection = document.createElement('div');
    reopenSection.className = 'start-screen-section';

    const reopenTitle = document.createElement('h3');
    reopenTitle.textContent = 'Reopen last session';
    reopenTitle.className = 'start-screen-section-title';
    reopenSection.appendChild(reopenTitle);

    const lastSessionDiv = document.createElement('div');
    lastSessionDiv.id = 'lastSessionFile';
    lastSessionDiv.className = 'start-screen-file-item';
    lastSessionDiv.textContent = 'No recent session';
    reopenSection.appendChild(lastSessionDiv);

    leftPanel.appendChild(reopenSection);

    // Recent projects section
    const recentSection = document.createElement('div');
    recentSection.className = 'start-screen-section';

    const recentTitle = document.createElement('h3');
    recentTitle.textContent = 'Recent projects';
    recentTitle.className = 'start-screen-section-title';
    recentSection.appendChild(recentTitle);

    const recentList = document.createElement('ul');
    recentList.id = 'recentProjectsList';
    recentList.className = 'start-screen-recent-list';
    recentSection.appendChild(recentList);

    leftPanel.appendChild(recentSection);

    return leftPanel;
}

function createRightPanel() {
    const rightPanel = document.createElement('div');
    rightPanel.className = 'start-screen-right-panel';

    const heading = document.createElement('h2');
    heading.textContent = 'Create a new project';
    heading.className = 'start-screen-heading';
    rightPanel.appendChild(heading);

    // Project focus options container
    const focusContainer = document.createElement('div');
    focusContainer.className = 'start-screen-focus-grid';

    const focusTypes = [
        {
            name: 'Animation',
            value: 'animation',
            iconSvg: '<svg width="80" height="80" viewBox="0 0 100 100" xmlns="http://www.w3.org/2000/svg"><path d="M20,80 Q30,60 50,50 T80,20" stroke="currentColor" stroke-width="4" fill="none" stroke-linecap="round"/><circle cx="30" cy="70" r="8" fill="currentColor"/></svg>',
            description: 'Drawing tools and timeline'
        },
        {
            name: 'Music',
            value: 'audioDaw',
            iconSvg: '<svg width="80" height="80" viewBox="0 0 100 100" xmlns="http://www.w3.org/2000/svg"><rect x="10" y="20" width="18" height="60" fill="none" stroke="currentColor" stroke-width="3"/><rect x="30" y="20" width="18" height="60" fill="none" stroke="currentColor" stroke-width="3"/><rect x="50" y="20" width="18" height="60" fill="none" stroke="currentColor" stroke-width="3"/><rect x="70" y="20" width="18" height="60" fill="none" stroke="currentColor" stroke-width="3"/><rect x="24" y="20" width="12" height="35" fill="currentColor"/><rect x="44" y="20" width="12" height="35" fill="currentColor"/><rect x="74" y="20" width="12" height="35" fill="currentColor"/></svg>',
            description: 'Audio tracks and mixer'
        },
        {
            name: 'Video editing',
            value: 'videoEditing',
            iconSvg: '<svg width="80" height="80" viewBox="0 0 100 100" xmlns="http://www.w3.org/2000/svg"><rect x="15" y="40" width="70" height="45" fill="currentColor" rx="4"/><rect x="15" y="25" width="70" height="15" fill="none" stroke="currentColor" stroke-width="3" rx="4"/><rect x="20" y="25" width="7" height="15" fill="currentColor"/><rect x="35" y="25" width="7" height="15" fill="currentColor"/><rect x="50" y="25" width="7" height="15" fill="currentColor"/><rect x="65" y="25" width="7" height="15" fill="currentColor"/></svg>',
            description: 'Clip timeline and effects'
        }
    ];

    focusTypes.forEach(focus => {
        const focusCard = createFocusCard(focus);
        focusContainer.appendChild(focusCard);
    });

    rightPanel.appendChild(focusContainer);

    return rightPanel;
}

function createFocusCard(focus) {
    const card = document.createElement('div');
    card.className = 'focus-card';

    // Icon container
    const iconContainer = document.createElement('div');
    iconContainer.className = 'focus-card-icon-container';

    const iconWrapper = document.createElement('div');
    iconWrapper.className = 'focus-card-icon';
    iconWrapper.innerHTML = focus.iconSvg;
    iconContainer.appendChild(iconWrapper);
    card.appendChild(iconContainer);

    // Label
    const label = document.createElement('div');
    label.textContent = focus.name;
    label.className = 'focus-card-label';
    card.appendChild(label);

    // Click handler
    card.addEventListener('click', () => {
        onProjectStartCallback({
            type: 'new',
            projectFocus: focus.value,
            width: 800,
            height: 600,
            fps: 24
        });
    });

    return card;
}

/**
 * Updates the recent files list and last session
 */
export async function updateStartScreen(config) {
    if (!startScreenContainer) return;

    // Update last session
    const lastSessionDiv = document.getElementById('lastSessionFile');
    if (lastSessionDiv) {
        if (config.recentFiles && config.recentFiles.length > 0) {
            const lastFile = config.recentFiles[0];
            const filename = await basename(lastFile);
            lastSessionDiv.textContent = filename;
            lastSessionDiv.onclick = () => {
                onProjectStartCallback({
                    type: 'reopen',
                    filePath: lastFile
                });
            };
            lastSessionDiv.classList.add('clickable');
        } else {
            lastSessionDiv.textContent = 'No recent session';
            lastSessionDiv.classList.remove('clickable');
            lastSessionDiv.onclick = null;
        }
    }

    // Update recent projects list
    const recentList = document.getElementById('recentProjectsList');
    if (recentList) {
        recentList.innerHTML = '';

        if (config.recentFiles && config.recentFiles.length > 1) {
            // Show up to 4 recent files (excluding the most recent which is shown as last session)
            const recentFiles = config.recentFiles.slice(1, 5);

            for (const filePath of recentFiles) {
                const filename = await basename(filePath);
                const listItem = document.createElement('li');
                listItem.textContent = filename;
                listItem.className = 'start-screen-file-item clickable';

                listItem.onclick = () => {
                    onProjectStartCallback({
                        type: 'recent',
                        filePath: filePath
                    });
                };

                recentList.appendChild(listItem);
            }
        }
    }
}

/**
 * Shows the start screen
 */
export function showStartScreen() {
    if (startScreenContainer) {
        startScreenContainer.style.display = 'flex';
    }
}

/**
 * Hides the start screen
 */
export function hideStartScreen() {
    if (startScreenContainer) {
        startScreenContainer.style.display = 'none';
    }
}
