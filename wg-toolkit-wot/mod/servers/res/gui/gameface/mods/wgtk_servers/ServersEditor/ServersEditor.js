// The servers' editor: the servers as its model lists them, `servers`, each with its
// `shortName`, `name` (its short name if empty), `address` and `key`; the one picked is changed or removed, by its index, the
// form making a new one when none is, `onSave` and `onRemove`.
//
// The client tells this page of no change to its model, which the game's own pages are
// told of: the page draws its list again where the model changes, after its commands and
// as it shows, its view filling the model as it is shown.

// The page's errors, in the client's log, see the view's `onLog`.
window.addEventListener('error', (event) => {
    if (window.model) {
        window.model.onLog({ message: `${event.message} (${event.filename}:${event.lineno})` });
    }
});

// How many frames after a command the page looks for its result, in the model.
const FOLLOW_FRAMES = 60;

let selected = -1;
// The frame the editor is to be measured in, none if it isn't.
let frame = 0;
// The frame the model is to be looked at in, none if it isn't.
let following = 0;
// The servers the list shows, as text.
let drawn = '';

// Size its window to the editor, two frames later, as the game's own pages do: the client
// ignores a size given before the page is laid out, then gives up after 3 s at a default
// size.
function resize() {
    window.cancelAnimationFrame(frame);
    frame = window.requestAnimationFrame(() => {
        frame = window.requestAnimationFrame(() => {
            frame = 0;
            const editor = field('editor');
            viewEnv.resizeViewPx(editor.offsetWidth, editor.offsetHeight);
        });
    });
}

function field(id) {
    return document.getElementById(id);
}

// The servers of the model, as plain objects.
function servers() {
    const list = window.model.servers;
    const result = [];
    for (let i = 0; i < list.length; i++) {
        const server = list[i].value;
        result.push({
            shortName: server.shortName,
            name: server.name,
            address: server.address,
            key: server.key,
        });
    }
    return result;
}

// Fill the form with a server, or empty it for a new one.
function pick(index) {
    const list = servers();
    selected = index >= 0 && index < list.length ? index : -1;
    const server = selected >= 0 ? list[selected] : { shortName: '', name: '', address: '', key: '' };
    field('shortName').value = server.shortName;
    field('name').value = server.name;
    field('address').value = server.address;
    field('key').value = server.key;
    field('remove').style.visibility = selected >= 0 ? 'visible' : 'hidden';
    // The form is for a new one already: no new one to make.
    field('new').classList.toggle('disabled', selected < 0);
    render();
}

// Draw the list again once the model's servers differ from it, after a command, looked
// at each frame for `FOLLOW_FRAMES` frames at most.
function follow() {
    let frames = FOLLOW_FRAMES;
    function look() {
        if (JSON.stringify(servers()) !== drawn) {
            following = 0;
            render();
        } else if (--frames > 0) {
            following = window.requestAnimationFrame(look);
        } else {
            following = 0;
        }
    }
    window.cancelAnimationFrame(following);
    following = window.requestAnimationFrame(look);
}

function render() {
    const list = field('list');
    list.innerHTML = '';
    const all = servers();
    drawn = JSON.stringify(all);
    if (all.length === 0) {
        const empty = document.createElement('div');
        empty.className = 'empty';
        empty.textContent = 'No server yet: fill the form below.';
        list.appendChild(empty);
    }
    all.forEach((server, index) => {
        const row = document.createElement('div');
        row.className = index === selected ? 'server selected' : 'server';
        const name = document.createElement('span');
        name.textContent = server.name || server.shortName;
        const address = document.createElement('span');
        address.className = 'address';
        address.textContent = server.address + (server.key ? '' : '  (no key)');
        row.appendChild(name);
        row.appendChild(address);
        row.addEventListener('click', () => pick(index));
        list.appendChild(row);
    });
    resize();
}

// Ready once its DOM is built, see the client's `gui/gameface/js/index.js`, and the
// engine is.
const ready = Promise.all([
    new Promise((resolve) => window.isDomBuilt ? resolve() : engine.on('self.onDomBuilt', resolve)),
    engine.whenReady,
]);

ready.then(() => {

    engine.on('self.onShowingStatusChanged', () => render());

    field('new').addEventListener('click', () => {
        if (selected >= 0) {
            pick(-1);
        }
    });
    field('save').addEventListener('click', () => {
        const address = field('address').value.trim();
        if (!address) {
            field('address').focus();
            return;
        }
        window.model.onSave({
            index: selected,
            shortName: field('shortName').value,
            name: field('name').value,
            address: address,
            key: field('key').value,
        });
        follow();
        // A new one added, the form is for the next.
        if (selected < 0) {
            pick(-1);
        }
    });
    field('remove').addEventListener('click', () => {
        if (selected >= 0) {
            window.model.onRemove({ index: selected });
            selected = -1;
            pick(-1);
            follow();
        }
    });
    field('close').addEventListener('click', () => window.model.onClose());

    pick(-1);

});
