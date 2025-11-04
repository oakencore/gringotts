use crate::evm;
use crate::solana;
use crate::storage::{Chain, WalletAddress};

fn format_usd(value: f64) -> String {
    let formatted = format!("{:.2}", value);
    let parts: Vec<&str> = formatted.split('.').collect();
    let integer_part = parts[0];
    let decimal_part = parts.get(1).unwrap_or(&"00");

    // Add commas to integer part
    let mut result = String::new();
    for (i, ch) in integer_part.chars().rev().enumerate() {
        if i > 0 && i % 3 == 0 {
            result.push(',');
        }
        result.push(ch);
    }

    format!("{}.{}", result.chars().rev().collect::<String>(), decimal_part)
}

pub fn render_addresses(addresses: &[WalletAddress]) {
    if addresses.is_empty() {
        println!("\nNo addresses tracked yet. Use 'gringotts add' to add addresses.\n");
        return;
    }

    println!("\n╭─────────────────────────────────────────────────────────────────────────────────────────────────────────╮");
    println!("│                                   TRACKED ADDRESSES                                                 │");
    println!("├─────────────────────────────────────────────────────────────────────────────────────────────────────────┤");
    println!("│ {:20} │ {:46} │ {:20} │", "Name", "Address", "Chain");
    println!("├─────────────────────────────────────────────────────────────────────────────────────────────────────────┤");

    for addr in addresses {
        let display_addr = if addr.address.len() > 46 {
            format!("{}...{}", &addr.address[..20], &addr.address[addr.address.len()-20..])
        } else {
            addr.address.clone()
        };
        println!("│ {:20} │ {:46} │ {:20} │", addr.name, display_addr, addr.chain.display_name());
    }

    println!("├─────────────────────────────────────────────────────────────────────────────────────────────────────────┤");
    println!("│ Total: {} address(es)                                                                                   │", addresses.len());
    println!("╰─────────────────────────────────────────────────────────────────────────────────────────────────────────╯\n");
}

pub fn render_solana_balances(name: &str, address: &str, balances: &solana::AccountBalances, chain: &Chain) {
    const MIN_WIDTH: usize = 79;

    // Collect all content lines to calculate max width
    let mut lines = Vec::new();

    // Header lines
    lines.push(format!("Wallet: {}", name));
    lines.push(format!("Address: {}", &address[..42.min(address.len())]));
    lines.push(format!("Chain: {}", chain.display_name()));

    // SOL Balance line
    let sol_line = if let Some(usd_value) = balances.sol_usd_value {
        if let Some(price) = balances.sol_usd_price {
            format!("SOL Balance: {:.9} SOL (${} @ ${})", balances.sol_balance, format_usd(usd_value), format_usd(price))
        } else {
            format!("SOL Balance: {:.9} SOL (${})", balances.sol_balance, format_usd(usd_value))
        }
    } else {
        format!("SOL Balance: {:.9} SOL", balances.sol_balance)
    };
    lines.push(sol_line);

    // Token balance lines
    if !balances.token_balances.is_empty() {
        lines.push("TOKEN BALANCES".to_string());
        for token in &balances.token_balances {
            let token_display = match (&token.name, &token.symbol) {
                (Some(name), Some(symbol)) => format!("{} ({})", name, symbol),
                (Some(name), None) => name.clone(),
                (None, Some(symbol)) => symbol.clone(),
                (None, None) => "Unknown Token".to_string(),
            };
            lines.push(token_display);

            let mint_display = if token.mint.len() > 44 {
                format!("    Mint: {}...{}", &token.mint[..20], &token.mint[token.mint.len()-20..])
            } else {
                format!("    Mint: {}", token.mint)
            };
            lines.push(mint_display);

            let balance_str = if let Some(usd_value) = token.usd_value {
                if let Some(price) = token.usd_price {
                    format!("    Balance: {:.6} (${} @ ${:.6})", token.ui_amount, format_usd(usd_value), price)
                } else {
                    format!("    Balance: {:.6} (${})", token.ui_amount, format_usd(usd_value))
                }
            } else {
                format!("    Balance: {:.6}", token.ui_amount)
            };
            lines.push(balance_str);
            lines.push(format!("    Decimals: {}", token.decimals));
        }
    }

    // Total USD Value line
    if let Some(total) = balances.total_usd_value {
        lines.push(format!("TOTAL USD VALUE: ${}", format_usd(total)));
    }

    // Calculate max width needed
    let max_content_width = lines.iter().map(|l| l.len()).max().unwrap_or(MIN_WIDTH);
    let box_width = max_content_width.max(MIN_WIDTH);

    // Top border
    println!("\n╔{}╗", "═".repeat(box_width + 2));

    // Header section
    println!("║  {:<width$} ║", lines[0], width = box_width);
    println!("║  {:<width$} ║", lines[1], width = box_width);
    println!("║  {:<width$} ║", lines[2], width = box_width);
    println!("╠{}╣", "═".repeat(box_width + 2));

    // SOL Balance
    println!("║  {:<width$} ║", lines[3], width = box_width);
    println!("╠{}╣", "═".repeat(box_width + 2));

    // Token Balances
    if balances.token_balances.is_empty() {
        println!("║  {:<width$} ║", "Token Balances: None", width = box_width);
    } else {
        println!("║  {:<width$} ║", lines[4], width = box_width);
        println!("╟{}╢", "─".repeat(box_width + 2));

        let mut line_idx = 5;
        for _ in &balances.token_balances {
            println!("║  {:<width$} ║", lines[line_idx], width = box_width);     // Token name
            println!("║  {:<width$} ║", lines[line_idx + 1], width = box_width); // Mint
            println!("║  {:<width$} ║", lines[line_idx + 2], width = box_width); // Balance
            println!("║  {:<width$} ║", lines[line_idx + 3], width = box_width); // Decimals
            println!("╟{}╢", "─".repeat(box_width + 2));
            line_idx += 4;
        }
    }

    // Total USD Value
    if balances.total_usd_value.is_some() {
        let total_line_idx = lines.len() - 1;
        println!("╠{}╣", "═".repeat(box_width + 2));
        println!("║  {:<width$} ║", lines[total_line_idx], width = box_width);
    }

    // Bottom border
    println!("╚{}╝\n", "═".repeat(box_width + 2));
}

pub fn render_error(error: &str) {
    println!("\n╭─────────────────────────────────────────────────────────────────────────────────╮");
    println!("│ ERROR                                                                            │");
    println!("├─────────────────────────────────────────────────────────────────────────────────┤");
    println!("│ {}                                                                              │", error);
    println!("╰─────────────────────────────────────────────────────────────────────────────────╯\n");
}

pub fn render_success(message: &str) {
    println!("\n{}\n", message);
}

pub fn render_evm_balances(name: &str, address: &str, balances: &evm::AccountBalances, chain: &Chain) {
    const MIN_WIDTH: usize = 79;

    // Collect all content lines to calculate max width
    let mut lines = Vec::new();

    // Header lines
    lines.push(format!("Wallet: {}", name));
    lines.push(format!("Address: {}", address));
    lines.push(format!("Chain: {}", chain.display_name()));

    // ETH Balance line
    let eth_line = if let Some(usd_value) = balances.eth_usd_value {
        if let Some(price) = balances.eth_usd_price {
            format!("ETH Balance: {:.9} ETH (${} @ ${})", balances.eth_balance, format_usd(usd_value), format_usd(price))
        } else {
            format!("ETH Balance: {:.9} ETH (${})", balances.eth_balance, format_usd(usd_value))
        }
    } else {
        format!("ETH Balance: {:.9} ETH", balances.eth_balance)
    };
    lines.push(eth_line);

    // Token balance lines
    if !balances.token_balances.is_empty() {
        lines.push("ERC20 TOKEN BALANCES".to_string());
        for token in &balances.token_balances {
            let token_display = match (&token.name, &token.symbol) {
                (Some(name), Some(symbol)) => format!("{} ({})", name, symbol),
                (Some(name), None) => name.clone(),
                (None, Some(symbol)) => symbol.clone(),
                (None, None) => "Unknown Token".to_string(),
            };
            lines.push(token_display);

            lines.push(format!("    Contract: {}", token.contract_address));

            let balance_str = if let Some(usd_value) = token.usd_value {
                if let Some(price) = token.usd_price {
                    format!("    Balance: {:.6} (${} @ ${:.6})", token.ui_amount, format_usd(usd_value), price)
                } else {
                    format!("    Balance: {:.6} (${})", token.ui_amount, format_usd(usd_value))
                }
            } else {
                format!("    Balance: {:.6}", token.ui_amount)
            };
            lines.push(balance_str);
            lines.push(format!("    Decimals: {}", token.decimals));
        }
    }

    // Total USD Value line
    if let Some(total) = balances.total_usd_value {
        lines.push(format!("TOTAL USD VALUE: ${}", format_usd(total)));
    }

    // Calculate max width needed
    let max_content_width = lines.iter().map(|l| l.len()).max().unwrap_or(MIN_WIDTH);
    let box_width = max_content_width.max(MIN_WIDTH);

    // Top border
    println!("\n╔{}╗", "═".repeat(box_width + 2));

    // Header section
    println!("║  {:<width$} ║", lines[0], width = box_width);
    println!("║  {:<width$} ║", lines[1], width = box_width);
    println!("║  {:<width$} ║", lines[2], width = box_width);
    println!("╠{}╣", "═".repeat(box_width + 2));

    // ETH Balance
    println!("║  {:<width$} ║", lines[3], width = box_width);
    println!("╠{}╣", "═".repeat(box_width + 2));

    // Token Balances
    if balances.token_balances.is_empty() {
        println!("║  {:<width$} ║", "Token Balances: None", width = box_width);
    } else {
        println!("║  {:<width$} ║", lines[4], width = box_width);
        println!("╟{}╢", "─".repeat(box_width + 2));

        let mut line_idx = 5;
        for _ in &balances.token_balances {
            println!("║  {:<width$} ║", lines[line_idx], width = box_width);     // Token name
            println!("║  {:<width$} ║", lines[line_idx + 1], width = box_width); // Contract
            println!("║  {:<width$} ║", lines[line_idx + 2], width = box_width); // Balance
            println!("║  {:<width$} ║", lines[line_idx + 3], width = box_width); // Decimals
            println!("╟{}╢", "─".repeat(box_width + 2));
            line_idx += 4;
        }
    }

    // Total USD Value
    if balances.total_usd_value.is_some() {
        let total_line_idx = lines.len() - 1;
        println!("╠{}╣", "═".repeat(box_width + 2));
        println!("║  {:<width$} ║", lines[total_line_idx], width = box_width);
    }

    // Bottom border
    println!("╚{}╝\n", "═".repeat(box_width + 2));
}

pub fn render_portfolio_summary(portfolio: &crate::PortfolioSummary) {
    const BOX_WIDTH: usize = 81;

    println!("\n╔═════════════════════════════════════════════════════════════════════════════════╗");
    println!("║                               PORTFOLIO SUMMARY                                 ║");
    println!("╠═════════════════════════════════════════════════════════════════════════════════╣");

    // Total Portfolio Value with proper padding
    let total_value_str = format!("Total Portfolio Value: ${}", format_usd(portfolio.total_usd_value));
    let total_value_len = total_value_str.len();
    let total_padding = if total_value_len < BOX_WIDTH - 2 { BOX_WIDTH - 2 - total_value_len } else { 0 };
    println!("║  {}{:width$} ║", total_value_str, "", width = total_padding);

    println!("╠═════════════════════════════════════════════════════════════════════════════════╣");
    println!("║  ASSETS BREAKDOWN                                                               ║");
    println!("╟─────────────────────────────────────────────────────────────────────────────────╢");

    if portfolio.assets.is_empty() {
        println!("║  No assets found                                                                ║");
    } else {
        // Sort assets by USD value (descending)
        let mut sorted_assets: Vec<_> = portfolio.assets.iter().collect();
        sorted_assets.sort_by(|a, b| b.1.total_usd_value.partial_cmp(&a.1.total_usd_value).unwrap());

        for (_, asset) in sorted_assets {
            // Symbol line
            let symbol_len = asset.symbol.len();
            let symbol_padding = if symbol_len < BOX_WIDTH - 2 { BOX_WIDTH - 2 - symbol_len } else { 0 };
            println!("║  {}{:width$} ║", asset.symbol, "", width = symbol_padding);

            // Amount line
            let amount_str = format!("Amount: {:.6}", asset.total_amount);
            let amount_len = amount_str.len();
            let amount_padding = if amount_len + 4 < BOX_WIDTH - 2 { BOX_WIDTH - 2 - amount_len - 4 } else { 0 };
            println!("║      {}{:width$} ║", amount_str, "", width = amount_padding);

            // USD Value line
            let usd_str = format!("USD Value: ${}", format_usd(asset.total_usd_value));
            let usd_len = usd_str.len();
            let usd_padding = if usd_len + 4 < BOX_WIDTH - 2 { BOX_WIDTH - 2 - usd_len - 4 } else { 0 };
            println!("║      {}{:width$} ║", usd_str, "", width = usd_padding);

            println!("╟─────────────────────────────────────────────────────────────────────────────────╢");
        }
    }

    println!("╚═════════════════════════════════════════════════════════════════════════════════╝\n");
}
