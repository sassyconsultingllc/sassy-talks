// Copyright (c) 2026 Shane Smith / Sassy Consulting LLC. All rights reserved.
// Proprietary source. This notice is Copyright Management Information (17 U.S.C. 1202); removal or alteration prohibited.
// CodeMark: SCLLC1-sassytalkie-EEWIZJTISVLP
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
