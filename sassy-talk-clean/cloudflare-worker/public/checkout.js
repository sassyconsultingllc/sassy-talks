async function buyProduct(product) {
    try {
        const response = await fetch('/api/checkout', {
            method: 'POST',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify({ product })
        });
        const data = await response.json();
        if (data.url) window.location.href = data.url;
    } catch (err) {
        alert('Checkout failed. Please try again.');
    }
}
