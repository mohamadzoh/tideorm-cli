/**
 * TideORM Studio - Frontend Application
 * Ocean-themed visual interface for TideORM CLI
 */

// State management
let configExists = true;
let pendingAction = null;
let pendingActionType = null;

// Initialize the application
document.addEventListener('DOMContentLoaded', function() {
    checkConfig();
    setupKeyboardShortcuts();
});

// Check if tideorm.toml exists
async function checkConfig() {
    try {
        const response = await fetch('/api/config-check');
        const data = await response.json();
        configExists = data.exists;
        
        if (!configExists) {
            showConfigWarning();
            disableCliFeatures();
        }
    } catch (error) {
        console.error('Failed to check config:', error);
        // Assume config doesn't exist on error
        configExists = false;
        showConfigWarning();
        disableCliFeatures();
    }
}

// Show warning banner when config is missing
function showConfigWarning() {
    const alert = document.getElementById('config-alert');
    if (alert) {
        alert.classList.remove('hidden');
    }
    
    // Update status badge
    const badge = document.getElementById('status-badge');
    if (badge) {
        badge.className = 'badge badge-warning';
        badge.textContent = '⚠ No Config';
    }
}

// Disable CLI features when config is missing
function disableCliFeatures() {
    // Disable all CLI-required elements
    document.querySelectorAll('.cli-required').forEach(el => {
        el.classList.add('disabled');
        el.onclick = function(e) {
            e.preventDefault();
            e.stopPropagation();
            showToast('warning', 'Configuration Required', 'Run "tideorm init" to create tideorm.toml first.');
        };
    });
    
    // Disable input fields and buttons
    document.querySelectorAll('.cli-input').forEach(el => {
        el.disabled = true;
    });
    
    document.querySelectorAll('.cli-btn').forEach(el => {
        el.disabled = true;
    });
}

// Panel switching
function switchPanel(panelName) {
    // Remove active class from all tabs and panels
    document.querySelectorAll('.nav-tab').forEach(tab => tab.classList.remove('active'));
    document.querySelectorAll('.panel').forEach(panel => panel.classList.remove('active'));
    
    // Add active class to selected tab and panel
    document.querySelector(`[data-panel="${panelName}"]`).classList.add('active');
    document.getElementById(`panel-${panelName}`).classList.add('active');
}

// Command execution
async function runCommand(command, displayName) {
    if (!configExists) {
        showToast('warning', 'Configuration Required', 'Run "tideorm init" to create tideorm.toml first.');
        return;
    }
    
    const outputId = getOutputIdForCommand(command);
    const outputEl = document.getElementById(outputId);
    
    if (outputEl) {
        outputEl.textContent = `Executing: tideorm ${command}\n\nPlease wait...`;
    }
    
    try {
        const response = await fetch('/api/execute', {
            method: 'POST',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify({ command })
        });
        
        const data = await response.json();
        
        if (outputEl) {
            outputEl.textContent = data.output || data.error || 'Command completed.';
        }
        
        if (data.success) {
            showToast('success', 'Success', `${displayName} completed successfully.`);
        } else {
            showToast('error', 'Error', data.error || 'Command failed.');
        }
    } catch (error) {
        if (outputEl) {
            outputEl.textContent = `Error: ${error.message}`;
        }
        showToast('error', 'Connection Error', 'Failed to communicate with the server.');
    }
}

// Get the appropriate output element ID based on command
function getOutputIdForCommand(command) {
    if (command.startsWith('make') || command.includes('generate')) {
        return 'generator-output';
    } else if (command.startsWith('migrate')) {
        return 'migration-output';
    } else if (command.startsWith('db seed') || command.includes('seeder')) {
        return 'seeder-output';
    } else if (command.startsWith('db')) {
        return 'database-output';
    }
    return 'generator-output';
}

// Model generation
function generateModel(event) {
    event.preventDefault();
    
    if (!configExists) {
        showToast('warning', 'Configuration Required', 'Run "tideorm init" to create tideorm.toml first.');
        return;
    }
    
    const command = buildModelCommand();
    runCommand(command, 'Model generation');
}

function buildModelCommand() {
    const name = document.getElementById('model-name').value;
    const table = document.getElementById('table-name').value;
    const fields = document.getElementById('fields').value;
    const relations = document.getElementById('relations').value;
    const indexes = document.getElementById('indexes').value;
    
    let command = `make model ${name}`;
    
    if (table) command += ` --table ${table}`;
    if (fields) command += ` --fields "${fields}"`;
    if (relations) command += ` --relations "${relations}"`;
    if (indexes) command += ` --indexed "${indexes}"`;
    
    if (!document.getElementById('opt-timestamps').checked) {
        command += ' --timestamps=false';
    }
    if (document.getElementById('opt-soft-delete').checked) {
        command += ' --soft-deletes';
    }
    if (document.getElementById('opt-migration').checked) {
        command += ' --migration';
    }
    if (document.getElementById('opt-factory').checked) {
        command += ' --factory';
    }
    if (document.getElementById('opt-seeder').checked) {
        command += ' --seeder';
    }
    
    return command;
}

function previewCommand() {
    const name = document.getElementById('model-name').value;
    if (!name) {
        showToast('warning', 'Model Name Required', 'Please enter a model name first.');
        return;
    }
    
    const command = buildModelCommand();
    const outputEl = document.getElementById('generator-output');
    outputEl.textContent = `Preview command:\n\ntideorm ${command}\n\n(Click "Generate Model" to execute)`;
}

// Other generators
function generateMigration() {
    const name = document.getElementById('migration-name').value;
    if (!name) {
        showToast('warning', 'Name Required', 'Please enter a migration name.');
        return;
    }
    runCommand(`make migration ${name}`, 'Migration creation');
}

function generateSeeder() {
    const name = document.getElementById('seeder-name').value;
    if (!name) {
        showToast('warning', 'Name Required', 'Please enter a seeder name.');
        return;
    }
    runCommand(`make seeder ${name}`, 'Seeder creation');
}

function generateFactory() {
    const name = document.getElementById('factory-name').value;
    if (!name) {
        showToast('warning', 'Name Required', 'Please enter a factory name.');
        return;
    }
    runCommand(`make factory ${name}`, 'Factory creation');
}

function runSpecificSeeder() {
    const name = document.getElementById('specific-seeder').value;
    if (!name) {
        showToast('warning', 'Name Required', 'Please enter a seeder name.');
        return;
    }
    runCommand(`db seed --class ${name}`, `Running ${name}`);
}

// Query playground
async function executeQuery() {
    const query = document.getElementById('query-input').value.trim();
    const resultsEl = document.getElementById('query-results');
    const timeEl = document.getElementById('query-time');
    
    if (!query) {
        showToast('warning', 'Query Required', 'Please enter a SQL query.');
        return;
    }
    
    // Check for dangerous queries
    if (isDangerousQuery(query)) {
        confirmDangerousQuery(query, 'Execute Dangerous Query', 
            'This query may modify or delete data. Are you sure you want to proceed?');
        return;
    }
    
    await executeQueryDirect(query);
}

async function executeQueryDirect(query) {
    const resultsEl = document.getElementById('query-results');
    const timeEl = document.getElementById('query-time');
    
    resultsEl.textContent = 'Executing query...';
    resultsEl.className = 'results-content';
    
    const startTime = performance.now();
    
    try {
        const response = await fetch('/api/query', {
            method: 'POST',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify({ query })
        });
        
        const data = await response.json();
        const endTime = performance.now();
        
        timeEl.textContent = `Executed in ${(endTime - startTime).toFixed(2)}ms`;
        
        if (data.success) {
            resultsEl.textContent = data.result || 'Query executed successfully. No results returned.';
            resultsEl.className = 'results-content success';
            showToast('success', 'Query Executed', 'Query completed successfully.');
        } else {
            resultsEl.textContent = `Error: ${data.error}`;
            resultsEl.className = 'results-content error';
            showToast('error', 'Query Failed', data.error);
        }
    } catch (error) {
        resultsEl.textContent = `Connection Error: ${error.message}`;
        resultsEl.className = 'results-content error';
        showToast('error', 'Connection Error', 'Failed to communicate with the server.');
    }
}

function clearQuery() {
    document.getElementById('query-input').value = '';
    document.getElementById('query-results').textContent = 'Results will appear here after executing a query.';
    document.getElementById('query-results').className = 'results-content';
    document.getElementById('query-time').textContent = '';
}

function setQuery(query) {
    document.getElementById('query-input').value = query;
}

// Check if a query is potentially dangerous
function isDangerousQuery(query) {
    const dangerousPatterns = [
        /\bDROP\b/i,
        /\bDELETE\b/i,
        /\bTRUNCATE\b/i,
        /\bALTER\b/i,
        /\bUPDATE\b.*\bWHERE\s+1\s*=\s*1/i,
        /\bDELETE\b.*\bWHERE\s+1\s*=\s*1/i
    ];
    
    return dangerousPatterns.some(pattern => pattern.test(query));
}

// Confirmation modal
function confirmDangerous(command, title, message) {
    if (!configExists) {
        showToast('warning', 'Configuration Required', 'Run "tideorm init" to create tideorm.toml first.');
        return;
    }
    
    pendingAction = command;
    pendingActionType = 'command';
    showModal(title, message);
}

function confirmDangerousQuery(query, title, message) {
    pendingAction = query;
    pendingActionType = 'query';
    showModal(title, message);
}

function showModal(title, message) {
    document.getElementById('modal-title').textContent = title;
    document.getElementById('modal-message').textContent = message;
    document.getElementById('confirm-modal').classList.add('active');
}

function closeModal() {
    document.getElementById('confirm-modal').classList.remove('active');
    pendingAction = null;
    pendingActionType = null;
}

function confirmModalAction() {
    const action = pendingAction;
    const type = pendingActionType;
    
    closeModal();
    
    if (type === 'command') {
        runCommand(action, action);
    } else if (type === 'query') {
        executeQueryDirect(action);
    }
}

// Toast notifications
function showToast(type, title, message) {
    const container = document.getElementById('toast-container');
    const toast = document.createElement('div');
    toast.className = `toast ${type}`;
    
    const icon = type === 'success' ? '✓' : type === 'error' ? '✗' : '⚠';
    
    toast.innerHTML = `
        <span style="font-size: 1.2rem;">${icon}</span>
        <div>
            <strong>${title}</strong>
            <p style="margin: 0; font-size: 0.85rem; opacity: 0.9;">${message}</p>
        </div>
        <button class="toast-close" onclick="this.parentElement.remove()">×</button>
    `;
    
    container.appendChild(toast);
    
    // Auto-remove after 5 seconds
    setTimeout(() => {
        if (toast.parentElement) {
            toast.style.animation = 'slideIn 0.3s ease reverse';
            setTimeout(() => toast.remove(), 300);
        }
    }, 5000);
}

// Keyboard shortcuts
function setupKeyboardShortcuts() {
    document.addEventListener('keydown', function(e) {
        // Ctrl/Cmd + Enter to execute query
        if ((e.ctrlKey || e.metaKey) && e.key === 'Enter') {
            const activePanel = document.querySelector('.panel.active');
            if (activePanel && activePanel.id === 'panel-query') {
                e.preventDefault();
                executeQuery();
            }
        }
        
        // Escape to close modal
        if (e.key === 'Escape') {
            closeModal();
        }
    });
}
