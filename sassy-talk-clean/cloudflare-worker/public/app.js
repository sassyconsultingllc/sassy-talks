/**
 * Sassy Consulting LLC - Frontend Application
 * Weather Dashboard + Network Analysis + VPN Detection
 */

document.addEventListener('DOMContentLoaded', () => {
    initializeDashboards();
    initializeDownloads();
});

function initializeDashboards() {
    // Weather Dashboard
    const zipInput = document.getElementById('zip-input');
    const analyzeBtn = document.getElementById('analyze-btn');
    
    // Network Dashboard
    const networkZipInput = document.getElementById('network-zip-input');
    const networkAnalyzeBtn = document.getElementById('network-analyze-btn');

    // Sync zip inputs
    zipInput?.addEventListener('input', (e) => {
        if (networkZipInput) networkZipInput.value = e.target.value;
    });
    networkZipInput?.addEventListener('input', (e) => {
        if (zipInput) zipInput.value = e.target.value;
    });

    // Weather analyze
    analyzeBtn?.addEventListener('click', () => {
        const zip = zipInput.value.trim();
        if (zip.length === 5) {
            fetchWeather(zip);
            fetchNetworkAnalysis(zip);
        }
    });

    // Network analyze
    networkAnalyzeBtn?.addEventListener('click', () => {
        const zip = networkZipInput.value.trim();
        if (zip.length === 5) {
            fetchWeather(zip);
            fetchNetworkAnalysis(zip);
        }
    });

    // Enter key support
    [zipInput, networkZipInput].forEach(input => {
        input?.addEventListener('keypress', (e) => {
            if (e.key === 'Enter') {
                const zip = e.target.value.trim();
                if (zip.length === 5) {
                    fetchWeather(zip);
                    fetchNetworkAnalysis(zip);
                }
            }
        });
    });

    // Load VPN recommendations on page load
    loadVPNRecommendations();
}

async function fetchWeather(zip) {
    const resultsDiv = document.getElementById('weather-results');
    const btn = document.getElementById('analyze-btn');
    
    btn.innerHTML = '<span class="loading"></span>';
    btn.disabled = true;

    try {
        const response = await fetch(`/api/weather?zip=${zip}`);
        const data = await response.json();

        if (data.error) {
            alert(data.error);
            return;
        }

        // Update weather display
        document.getElementById('weather-city').textContent = `${data.location.city}, ${data.location.state}`;
        document.getElementById('weather-date').textContent = new Date().toLocaleDateString('en-US', { 
            weekday: 'long', year: 'numeric', month: 'long', day: 'numeric' 
        });
        document.getElementById('current-temp').textContent = Math.round(data.current.temperature_2m);
        document.getElementById('feels-like').textContent = `${Math.round(data.current.apparent_temperature)}°F`;
        document.getElementById('humidity').textContent = `${data.current.relative_humidity_2m}%`;
        document.getElementById('wind').textContent = `${Math.round(data.current.wind_speed_10m)} mph`;

        // Build forecast
        const forecastDiv = document.getElementById('forecast');
        forecastDiv.innerHTML = '';
        
        const days = ['Sun', 'Mon', 'Tue', 'Wed', 'Thu', 'Fri', 'Sat'];
        for (let i = 0; i < Math.min(5, data.daily.time.length); i++) {
            const date = new Date(data.daily.time[i]);
            const dayName = days[date.getDay()];
            const high = Math.round(data.daily.temperature_2m_max[i]);
            const low = Math.round(data.daily.temperature_2m_min[i]);
            const icon = getWeatherIcon(data.daily.weather_code[i]);
            
            forecastDiv.innerHTML += `
                <div class="forecast-day">
                    <div class="day">${dayName}</div>
                    <div class="icon">${icon}</div>
                    <div class="temp">${high}° / ${low}°</div>
                </div>
            `;
        }

        resultsDiv.classList.remove('hidden');
    } catch (error) {
        console.error('Weather fetch error:', error);
        alert('Failed to fetch weather data');
    } finally {
        btn.innerHTML = 'Analyze';
        btn.disabled = false;
    }
}

function getWeatherIcon(code) {
    // WMO Weather codes to emoji
    const icons = {
        0: '☀️',   // Clear sky
        1: '🌤️',  // Mainly clear
        2: '⛅',   // Partly cloudy
        3: '☁️',   // Overcast
        45: '🌫️', // Fog
        48: '🌫️', // Depositing rime fog
        51: '🌧️', // Light drizzle
        53: '🌧️', // Moderate drizzle
        55: '🌧️', // Dense drizzle
        61: '🌧️', // Slight rain
        63: '🌧️', // Moderate rain
        65: '🌧️', // Heavy rain
        71: '🌨️', // Slight snow
        73: '🌨️', // Moderate snow
        75: '❄️',  // Heavy snow
        77: '🌨️', // Snow grains
        80: '🌦️', // Slight rain showers
        81: '🌦️', // Moderate rain showers
        82: '⛈️',  // Violent rain showers
        85: '🌨️', // Slight snow showers
        86: '🌨️', // Heavy snow showers
        95: '⛈️',  // Thunderstorm
        96: '⛈️',  // Thunderstorm with hail
        99: '⛈️'   // Thunderstorm with heavy hail
    };
    return icons[code] || '🌤️';
}

async function fetchNetworkAnalysis(zip) {
    const resultsDiv = document.getElementById('network-results');
    const btn = document.getElementById('network-analyze-btn');
    
    btn.innerHTML = '<span class="loading"></span>';
    btn.disabled = true;

    try {
        const response = await fetch('/api/analyze', {
            method: 'POST',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify({ zipCode: zip })
        });
        
        const data = await response.json();

        // Update connection status
        const statusBanner = document.getElementById('connection-status');
        if (data.connection.verified) {
            statusBanner.classList.remove('warning');
            statusBanner.querySelector('.status-icon').textContent = '✓';
            statusBanner.querySelector('strong').textContent = 'Connection Verified';
            statusBanner.querySelector('span').textContent = 'No suspicious patterns detected';
        } else {
            statusBanner.classList.add('warning');
            statusBanner.querySelector('.status-icon').textContent = '!';
            statusBanner.querySelector('strong').textContent = 'Connection Warning';
            statusBanner.querySelector('span').textContent = 'Suspicious patterns detected';
        }

        // Update network cards
        document.getElementById('ip-address').textContent = data.ip.address;
        document.getElementById('ip-sub').textContent = data.network.asn || 'Unknown';
        
        document.getElementById('location').textContent = data.location.city !== 'Unknown' ? 
            data.location.city : 'Unknown';
        document.getElementById('location-sub').textContent = data.location.country !== 'Unknown' ? 
            data.location.country : 'Unknown';
        
        document.getElementById('isp').textContent = data.network.isp !== 'Unknown' ? 
            truncateText(data.network.isp, 20) : 'Unknown';
        document.getElementById('isp-sub').textContent = data.network.isp !== 'Unknown' ? 
            'Identified' : 'Unknown';
        
        document.getElementById('ping').textContent = data.performance.ping;

        // Update VPN banner
        const vpnBanner = document.getElementById('vpn-banner');
        const vpnIcon = document.getElementById('vpn-icon');
        const vpnStatus = document.getElementById('vpn-status');
        const vpnMessage = document.getElementById('vpn-message');
        const vpnRecommendations = document.getElementById('vpn-recommendations');

        if (data.vpn.detected) {
            vpnBanner.classList.add('detected');
            vpnIcon.textContent = '🔒';
            vpnStatus.textContent = `VPN Detection: Detected (${data.vpn.provider || 'Unknown provider'})`;
            vpnMessage.textContent = data.vpn.message;
            vpnRecommendations.classList.add('hidden');
        } else {
            vpnBanner.classList.remove('detected');
            vpnIcon.textContent = '🛡️';
            vpnStatus.textContent = 'VPN Detection: Not Detected';
            vpnMessage.textContent = data.vpn.message;
            vpnRecommendations.classList.remove('hidden');
        }

        // Update geographic details
        document.getElementById('geo-postal').textContent = data.location.postalCode || 'N/A';
        document.getElementById('geo-coords').textContent = 
            `${data.location.coordinates.latitude.toFixed(4)}, ${data.location.coordinates.longitude.toFixed(4)}`;
        document.getElementById('geo-country').textContent = data.location.country || 'Unknown';

        // Update connection details
        document.getElementById('conn-timezone').textContent = data.timezone || 'Unknown';
        document.getElementById('conn-zipcode').textContent = data.inputZipCode || '--';
        document.getElementById('conn-region').textContent = data.location.region || 'N/A';

        resultsDiv.classList.remove('hidden');
    } catch (error) {
        console.error('Network analysis error:', error);
        // Show results anyway with mock data for demo
        showMockNetworkResults(zip);
    } finally {
        btn.innerHTML = 'Analyze';
        btn.disabled = false;
    }
}

function showMockNetworkResults(zip) {
    // Fallback for demo when API not available
    const resultsDiv = document.getElementById('network-results');
    
    document.getElementById('ip-address').textContent = '69.131.106.44';
    document.getElementById('ip-sub').textContent = 'AS7922';
    document.getElementById('location').textContent = 'Unknown';
    document.getElementById('location-sub').textContent = 'Unknown';
    document.getElementById('isp').textContent = 'Unknown';
    document.getElementById('isp-sub').textContent = 'Unknown';
    document.getElementById('ping').textContent = '189';
    
    document.getElementById('geo-postal').textContent = 'N/A';
    document.getElementById('geo-coords').textContent = '0.0000, 0.0000';
    document.getElementById('geo-country').textContent = 'Unknown';
    document.getElementById('conn-timezone').textContent = 'Unknown';
    document.getElementById('conn-zipcode').textContent = zip;
    document.getElementById('conn-region').textContent = 'N/A';

    resultsDiv.classList.remove('hidden');
}

async function loadVPNRecommendations() {
    const vpnOptions = document.getElementById('vpn-options');
    
    try {
        const response = await fetch('/api/vpn-recommendations');
        const data = await response.json();
        
        if (data.recommendations && data.recommendations.length > 0) {
            renderVPNOptions(data.recommendations);
        } else {
            renderDefaultVPNOptions();
        }
    } catch (error) {
        renderDefaultVPNOptions();
    }
}

function renderVPNOptions(recommendations) {
    const vpnOptions = document.getElementById('vpn-options');
    vpnOptions.innerHTML = recommendations.map(vpn => `
        <a href="${vpn.website}" target="_blank" rel="noopener" class="vpn-option">
            <strong>${vpn.name}<span class="free-badge">FREE</span></strong>
            <span>Click to learn more</span>
        </a>
    `).join('');
}

function renderDefaultVPNOptions() {
    const defaults = [
        { name: 'ProtonVPN', website: 'https://protonvpn.com' },
        { name: 'Windscribe', website: 'https://windscribe.com' },
        { name: 'Cloudflare WARP', website: 'https://1.1.1.1' }
    ];
    renderVPNOptions(defaults);
}

function truncateText(text, maxLength) {
    if (text.length <= maxLength) return text;
    return text.substring(0, maxLength) + '...';
}

// Download functionality
function initializeDownloads() {
    document.querySelectorAll('.download-btn').forEach(btn => {
        btn.addEventListener('click', (e) => {
            e.preventDefault();
            e.stopPropagation();
            const product = btn.dataset.product;
            showDownloadModal(product);
        });
    });
}

function showDownloadModal(product) {
    const products = {
        'sassy-talk': {
            name: 'Sassy-Talk',
            platforms: [
                { name: 'Android APK', file: 'sassy-talk-v1.0.0.apk', platform: 'android' },
                { name: 'Windows MSI', file: 'sassy-talk-v1.0.0.msi', platform: 'windows' }
            ]
        },
        'winforensics': {
            name: 'WinForensics',
            platforms: [
                { name: 'Windows MSI', file: 'winforensics-v1.0.0.msi', platform: 'windows' }
            ]
        }
    };

    const info = products[product];
    if (!info) return;

    const modal = document.createElement('div');
    modal.className = 'download-modal';
    modal.innerHTML = `
        <div class="modal-backdrop"></div>
        <div class="modal-content">
            <h3>Download ${info.name}</h3>
            <div class="platform-options">
                ${info.platforms.map(p => `
                    <a href="/download/${product}/${p.platform}/${p.file}" class="platform-option">
                        ${p.platform === 'android' ? '📱' : '💻'} ${p.name}
                    </a>
                `).join('')}
            </div>
            <button class="btn btn-secondary close-modal">Cancel</button>
        </div>
    `;
    
    document.body.appendChild(modal);
    
    modal.querySelector('.modal-backdrop').addEventListener('click', () => modal.remove());
    modal.querySelector('.close-modal').addEventListener('click', () => modal.remove());
}

// Add modal styles dynamically
const modalStyles = document.createElement('style');
modalStyles.textContent = `
    .download-modal {
        position: fixed;
        inset: 0;
        z-index: 9999;
        display: flex;
        align-items: center;
        justify-content: center;
    }
    .modal-backdrop {
        position: absolute;
        inset: 0;
        background: rgba(0,0,0,0.7);
    }
    .modal-content {
        position: relative;
        background: var(--bg-secondary);
        padding: 2rem;
        border-radius: 16px;
        max-width: 400px;
        width: 90%;
        text-align: center;
    }
    .modal-content h3 {
        margin-bottom: 1.5rem;
    }
    .platform-options {
        display: flex;
        flex-direction: column;
        gap: 1rem;
        margin-bottom: 1.5rem;
    }
    .platform-option {
        display: block;
        padding: 1rem;
        background: var(--bg-tertiary);
        border: 1px solid var(--border-subtle);
        border-radius: 8px;
        color: var(--text-primary);
        text-decoration: none;
        transition: all 0.2s;
    }
    .platform-option:hover {
        border-color: var(--accent-primary);
        background: var(--bg-card);
    }
`;
document.head.appendChild(modalStyles);
