/**
 * Sassy Browser Glossary System
 * Adds blue squiggly underlines to technical terms with hover/click tooltips
 * Terms get a ˢᵘᵖᵉʳˢᶜʳⁱᵖᵗ ? indicator
 * 
 * Usage: Add class="term" and data-term="term-key" to any element
 * Example: <span class="term" data-term="rust">Rust</span>
 */

const glossary = {
    // Programming & Technology
    "rust": {
        title: "Rust",
        definition: "A modern programming language focused on safety, speed, and preventing crashes. Unlike older languages, it catches common bugs before your code even runs.",
        source: "rust-lang.org",
        url: "https://www.rust-lang.org/"
    },
    "cpp": {
        title: "C++",
        definition: "A powerful but older programming language (1985). Most browsers are written in it. The problem? It's easy to write bugs that hackers can exploit—memory leaks, buffer overflows, and security holes.",
        source: "Wikipedia",
        url: "https://en.wikipedia.org/wiki/C%2B%2B"
    },
    "fork": {
        title: "Fork (Software)",
        definition: "A copy of someone else's code that you modify independently. Brave, Edge, and Opera are all 'forks' of Chrome—they inherited Chrome's codebase (and its problems). Sassy is NOT a fork.",
        source: "GitHub Docs",
        url: "https://docs.github.com/en/get-started/quickstart/fork-a-repo"
    },
    "sandbox": {
        title: "Sandbox",
        definition: "An isolated environment where programs run with restricted permissions. Like a playpen for code—if something malicious happens inside, it can't escape to affect the rest of your computer.",
        source: "Wikipedia",
        url: "https://en.wikipedia.org/wiki/Sandbox_(computer_security)"
    },
    "telemetry": {
        title: "Telemetry",
        definition: "Data about how you use software that gets secretly sent back to the company. This often includes every website you visit, what you click, how long you stay—basically spying disguised as 'improving the product.'",
        source: "Mozilla",
        url: "https://support.mozilla.org/en-US/kb/telemetry-clientid"
    },
    "chromium": {
        title: "Chromium",
        definition: "Google's open-source browser engine. Chrome, Edge, Brave, and Opera are all based on it—meaning they all share Google's code, Google's tracking infrastructure, and many of the same vulnerabilities.",
        source: "chromium.org",
        url: "https://www.chromium.org/"
    },
    "v8": {
        title: "V8 JavaScript Engine",
        definition: "Google's JavaScript engine used in Chrome. It uses JIT compilation which is fast but historically has been a major source of security vulnerabilities that hackers love to exploit.",
        source: "v8.dev",
        url: "https://v8.dev/"
    },
    "jit": {
        title: "JIT (Just-In-Time) Compilation",
        definition: "A technique that compiles code while the program runs for better speed. Unfortunately, it's a favorite target for hackers—JIT compilers have been exploited countless times to run malicious code.",
        source: "OWASP",
        url: "https://owasp.org/www-community/attacks/JIT_Spraying"
    },

    // Encryption & Security
    "chacha20": {
        title: "ChaCha20-Poly1305",
        definition: "A modern encryption algorithm used by WireGuard VPN, Google, and Cloudflare. Think of it as an unbreakable lock—your data gets scrambled so thoroughly that even supercomputers can't crack it.",
        source: "IETF RFC 8439",
        url: "https://datatracker.ietf.org/doc/html/rfc8439"
    },
    "ed25519": {
        title: "Ed25519 Key Pair",
        definition: "A digital signature system used by Signal, SSH, and Tor. It creates a 'public key' (your identity) and 'private key' (your secret). Together they prove you are who you say you are—without ever revealing your secret.",
        source: "ed25519.cr.yp.to",
        url: "https://ed25519.cr.yp.to/"
    },
    "argon2": {
        title: "Argon2id",
        definition: "Winner of the Password Hashing Competition (2015). It's the gold standard for protecting stored passwords—deliberately slow and memory-intensive, making it nearly impossible for hackers to crack even simple passwords.",
        source: "OWASP",
        url: "https://cheatsheetseries.owasp.org/cheatsheets/Password_Storage_Cheat_Sheet.html"
    },
    "entropy": {
        title: "Entropy",
        definition: "Randomness collected from unpredictable sources—your mouse movements, typing patterns, timing. This randomness is essential for generating encryption keys that nobody could possibly guess.",
        source: "Wikipedia",
        url: "https://en.wikipedia.org/wiki/Entropy_(computing)"
    },
    "e2e": {
        title: "End-to-End Encryption",
        definition: "Encryption where only you and the recipient can read messages. Not the government. Not hackers. Not even the company running the service. They only see scrambled gibberish.",
        source: "EFF",
        url: "https://ssd.eff.org/module/communicating-others#end-to-end-encryption"
    },
    "keypair": {
        title: "Cryptographic Key Pair",
        definition: "Two mathematically linked keys: a PUBLIC key (share freely, like an address) and a PRIVATE key (keep secret, like a password). Used together, they enable secure communication and prove your identity.",
        source: "Wikipedia",
        url: "https://en.wikipedia.org/wiki/Public-key_cryptography"
    },

    // Networking
    "tailscale": {
        title: "Tailscale",
        definition: "A mesh VPN that connects your devices directly to each other—no middle-man servers. Your phone talks to your laptop through an encrypted tunnel. Your data never touches anyone else's servers.",
        source: "tailscale.com",
        url: "https://tailscale.com/kb/1151/what-is-tailscale"
    },
    "oauth": {
        title: "OAuth",
        definition: "A secure way to log into websites using another account (like 'Sign in with Google'). It lets you authenticate without giving the website your actual password—they just get proof you logged in elsewhere.",
        source: "oauth.net",
        url: "https://oauth.net/2/"
    },
    "captcha": {
        title: "CAPTCHA",
        definition: "Those 'select all the traffic lights' puzzles. They verify you're human, not a bot. Fun fact: some CAPTCHAs also track you, which is why Sassy's popup blocker allows them while blocking other popups.",
        source: "Wikipedia",
        url: "https://en.wikipedia.org/wiki/CAPTCHA"
    },

    // Browser Security
    "clickjacking": {
        title: "Clickjacking",
        definition: "A sneaky attack where invisible buttons are placed over real ones. You think you're clicking 'Play Video' but you're actually clicking 'Transfer $1000' or 'Allow Webcam'. Sassy blocks tiny invisible elements to prevent this.",
        source: "OWASP",
        url: "https://owasp.org/www-community/attacks/Clickjacking"
    },
    "quarantine": {
        title: "Download Quarantine",
        definition: "Downloaded files are held in a secure waiting area (in memory, not on disk) until you explicitly approve them multiple times. This stops 'drive-by downloads' from secretly installing malware.",
        source: "Sassy Browser",
        url: "#"
    },
    "malware": {
        title: "Malware",
        definition: "Malicious software designed to harm your computer or steal your data. Includes viruses, ransomware (locks your files for ransom), spyware, and trojans. Often installed accidentally through deceptive downloads.",
        source: "CISA",
        url: "https://www.cisa.gov/news-events/news/what-malware"
    },
    "nsfw": {
        title: "NSFW Detection",
        definition: "Automatic detection of adult content using image analysis. In Sassy Browser, NSFW pages are excluded from history and sync—protecting your privacy (and keeping inappropriate content away from kids).",
        source: "Sassy Browser",
        url: "#"
    },

    // File Formats
    "pdb": {
        title: "PDB (Protein Data Bank)",
        definition: "A file format for 3D molecular structures—proteins, DNA, drug compounds. Used by researchers and students worldwide. Normally requires expensive software like ChemDraw ($2,600/year) to view.",
        source: "RCSB PDB",
        url: "https://www.rcsb.org/"
    },
    "raw": {
        title: "RAW Image Files",
        definition: "Unprocessed image data straight from your camera sensor. Contains much more detail than JPEG but requires special software to view. Sassy opens CR2, NEF, ARW, DNG, and dozens more formats free.",
        source: "Wikipedia",
        url: "https://en.wikipedia.org/wiki/Raw_image_format"
    },
    "dicom": {
        title: "DICOM",
        definition: "The standard format for medical images (X-rays, MRIs, CT scans). Hospital viewing software (PACS) typically costs thousands of dollars. Sassy opens them for free.",
        source: "DICOM Standard",
        url: "https://www.dicomstandard.org/"
    }
};

// Initialize tooltips when DOM is ready
document.addEventListener('DOMContentLoaded', function() {
    const terms = document.querySelectorAll('.term');
    
    terms.forEach(term => {
        const key = term.getAttribute('data-term');
        const entry = glossary[key];
        
        if (!entry) {
            console.warn(`Glossary entry not found for: ${key}`);
            return;
        }
        
        // Add superscript question mark indicator
        const indicator = document.createElement('sup');
        indicator.className = 'term-indicator';
        indicator.textContent = '?';
        term.appendChild(indicator);
        
        // Create tooltip element
        const tooltip = document.createElement('div');
        tooltip.className = 'term-tooltip';
        tooltip.innerHTML = `
            <strong>${entry.title}</strong>
            <p>${entry.definition}</p>
            <span class="term-source">
                Source: <a href="${entry.url}" target="_blank" rel="noopener">${entry.source}</a>
            </span>
        `;
        
        term.appendChild(tooltip);
        
        // Make focusable for accessibility
        term.setAttribute('tabindex', '0');
        term.setAttribute('role', 'button');
        term.setAttribute('aria-label', `Learn about ${entry.title}`);
    });
});

// Add CSS for terms
const style = document.createElement('style');
style.textContent = `
    .term {
        position: relative;
        cursor: help;
        text-decoration: underline;
        text-decoration-style: wavy;
        text-decoration-color: #3b82f6;
        text-underline-offset: 3px;
    }
    .term:hover { color: #3b82f6; }
    .term-indicator {
        color: #3b82f6;
        font-size: 0.65em;
        font-weight: bold;
        margin-left: 1px;
        vertical-align: super;
    }
    .term-tooltip {
        position: absolute;
        bottom: calc(100% + 12px);
        left: 50%;
        transform: translateX(-50%);
        background: #1a1a1a;
        border: 1px solid #3b82f6;
        border-radius: 8px;
        padding: 12px 16px;
        width: 300px;
        opacity: 0;
        visibility: hidden;
        transition: all 0.2s ease;
        z-index: 1000;
        box-shadow: 0 4px 20px rgba(0,0,0,0.5);
        pointer-events: none;
        text-decoration: none;
    }
    .term:hover .term-tooltip,
    .term:focus .term-tooltip {
        opacity: 1;
        visibility: visible;
        pointer-events: auto;
    }
    .term-tooltip::after {
        content: '';
        position: absolute;
        top: 100%;
        left: 50%;
        transform: translateX(-50%);
        border: 8px solid transparent;
        border-top-color: #3b82f6;
    }
    .term-tooltip strong {
        display: block;
        color: #3b82f6;
        font-size: 0.8rem;
        text-transform: uppercase;
        letter-spacing: 0.05em;
        margin-bottom: 6px;
    }
    .term-tooltip p {
        color: #e0e0e0;
        font-size: 0.85rem;
        line-height: 1.5;
        margin: 0;
        text-decoration: none;
    }
    .term-tooltip .term-source {
        display: block;
        margin-top: 8px;
        font-size: 0.7rem;
        color: #808080;
    }
    .term-tooltip .term-source a {
        color: #3b82f6;
        text-decoration: none;
    }
    .term-tooltip .term-source a:hover {
        text-decoration: underline;
    }
    @media (max-width: 768px) {
        .term-tooltip {
            width: 260px;
            left: 0;
            transform: none;
        }
        .term-tooltip::after { left: 20px; }
    }
`;
document.head.appendChild(style);
