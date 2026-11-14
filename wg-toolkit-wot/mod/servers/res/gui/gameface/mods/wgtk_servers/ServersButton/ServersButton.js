// The button: sized to itself, it tells its view it was clicked.

// Size its window two frames after the page is ready, as the game's own pages do: the
// client ignores a size given before, then gives up after 3 s at a default size. A frame
// later, its view is told, for the window to be placed at its new size.
function resize() {
    window.requestAnimationFrame(() => {
        window.requestAnimationFrame(() => {
            viewEnv.resizeViewRem(140, 40);
            window.requestAnimationFrame(() => window.model.onSized());
        });
    });
}

// Ready once its DOM is built, see the client's `gui/gameface/js/index.js`, and the
// engine is.
const ready = Promise.all([
    new Promise((resolve) => window.isDomBuilt ? resolve() : engine.on('self.onDomBuilt', resolve)),
    engine.whenReady,
]);

ready.then(() => {

    resize();

    document.getElementById('servers').addEventListener('click', () => {
        engine.call('PlaySound', 'play');
        window.model.onClick();
    });

});
