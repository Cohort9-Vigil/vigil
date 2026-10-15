// Dedicated Web3 Wallet State and Connection Provider for Vigil Protocol on Ethereum Sepolia.
import React, { createContext, useContext, useEffect, useState, useCallback } from "react";
import { ethers } from "ethers";
import { provider, RPC_URL } from "../lib/chain.ts";

export const SEPOLIA_CHAIN_ID = 11155111;
export const SEPOLIA_HEX = "0xaa36a7";

export interface WalletContextType {
  account: string | null;
  chainId: number | null;
  isSepolia: boolean;
  isConnecting: boolean;
  hasInjected: boolean;
  ethBalance: string | null;
  error: string | null;
  isDemoMode: boolean;
  connect: () => Promise<void>;
  disconnect: () => void;
  switchNetwork: () => Promise<boolean>;
  inspectAddress: (address: string) => void;
  clearError: () => void;
}

const WalletContext = createContext<WalletContextType | null>(null);

const STORAGE_KEY = "vigil_wallet_connected";
const DEMO_KEY = "vigil_demo_address";

export function WalletProvider({ children }: { children: React.ReactNode }) {
  const [account, setAccount] = useState<string | null>(null);
  const [chainId, setChainId] = useState<number | null>(null);
  const [isConnecting, setIsConnecting] = useState(false);
  const [ethBalance, setEthBalance] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [isDemoMode, setIsDemoMode] = useState(false);

  const hasInjected = typeof window !== "undefined" && Boolean((window as unknown as { ethereum?: unknown }).ethereum);

  // Fetch native Sepolia ETH balance for the active account
  const refreshBalance = useCallback(async (addr: string) => {
    try {
      const bal = await provider.getBalance(addr);
      setEthBalance(Number(ethers.formatEther(bal)).toFixed(4));
    } catch {
      setEthBalance(null);
    }
  }, []);

  // Switch or prompt to add Sepolia network
  const switchNetwork = useCallback(async (): Promise<boolean> => {
    if (!hasInjected) return false;
    const eth = (window as unknown as { ethereum: { request: (args: unknown) => Promise<unknown> } }).ethereum;
    try {
      await eth.request({
        method: "wallet_switchEthereumChain",
        params: [{ chainId: SEPOLIA_HEX }],
      });
      return true;
    } catch (err: unknown) {
      const code = (err as { code?: number })?.code;
      if (code === 4902) {
        try {
          await eth.request({
            method: "wallet_addEthereumChain",
            params: [
              {
                chainId: SEPOLIA_HEX,
                chainName: "Ethereum Sepolia",
                nativeCurrency: { name: "Sepolia ETH", symbol: "ETH", decimals: 18 },
                rpcUrls: [RPC_URL],
                blockExplorerUrls: ["https://sepolia.etherscan.io"],
              },
            ],
          });
          return true;
        } catch {
          return false;
        }
      }
      return false;
    }
  }, [hasInjected]);

  // Connect to MetaMask or injected Web3 wallet
  const connect = useCallback(async () => {
    setError(null);
    if (!hasInjected) {
      setError("No Web3 wallet detected. Please install MetaMask or use Explore Address mode.");
      return;
    }
    setIsConnecting(true);
    try {
      const eth = (window as unknown as { ethereum: { request: (args: unknown) => Promise<unknown> } }).ethereum;
      const accounts = (await eth.request({ method: "eth_requestAccounts" })) as string[];
      if (!accounts || accounts.length === 0) {
        throw new Error("No accounts selected in wallet.");
      }
      const rawChain = (await eth.request({ method: "eth_chainId" })) as string;
      const currentChain = parseInt(rawChain, 16);
      setChainId(currentChain);

      if (currentChain !== SEPOLIA_CHAIN_ID) {
        await switchNetwork();
        const updatedChain = (await eth.request({ method: "eth_chainId" })) as string;
        setChainId(parseInt(updatedChain, 16));
      }

      const activeAccount = accounts[0].toLowerCase();
      setAccount(activeAccount);
      setIsDemoMode(false);
      localStorage.setItem(STORAGE_KEY, "true");
      localStorage.removeItem(DEMO_KEY);
      await refreshBalance(activeAccount);
    } catch (err: unknown) {
      const msg = (err as { message?: string })?.message ?? "Failed to connect wallet";
      setError(msg);
    } finally {
      setIsConnecting(false);
    }
  }, [hasInjected, switchNetwork, refreshBalance]);

  const disconnect = useCallback(() => {
    setAccount(null);
    setEthBalance(null);
    setIsDemoMode(false);
    localStorage.removeItem(STORAGE_KEY);
    localStorage.removeItem(DEMO_KEY);
  }, []);

  // Inspect any address in read-only mode without needing a wallet
  const inspectAddress = useCallback((addr: string) => {
    const trimmed = addr.trim().toLowerCase();
    if (!ethers.isAddress(trimmed)) {
      setError("Invalid Ethereum address");
      return;
    }
    setAccount(trimmed);
    setIsDemoMode(true);
    setError(null);
    localStorage.setItem(DEMO_KEY, trimmed);
    refreshBalance(trimmed);
  }, [refreshBalance]);

  // Auto-connect on page load if previously connected or if demo address was saved
  useEffect(() => {
    const savedDemo = localStorage.getItem(DEMO_KEY);
    if (savedDemo && ethers.isAddress(savedDemo)) {
      setAccount(savedDemo);
      setIsDemoMode(true);
      refreshBalance(savedDemo);
      return;
    }

    if (!hasInjected) return;
    const eth = (window as unknown as {
      ethereum: {
        request: (args: unknown) => Promise<unknown>;
        on?: (event: string, handler: (...args: unknown[]) => void) => void;
        removeListener?: (event: string, handler: (...args: unknown[]) => void) => void;
      };
    }).ethereum;

    const wasConnected = localStorage.getItem(STORAGE_KEY) === "true";
    if (wasConnected) {
      eth.request({ method: "eth_accounts" })
        .then((accs) => {
          const list = accs as string[];
          if (list && list.length > 0) {
            const active = list[0].toLowerCase();
            setAccount(active);
            setIsDemoMode(false);
            refreshBalance(active);
            eth.request({ method: "eth_chainId" }).then((c) => {
              setChainId(parseInt(c as string, 16));
            });
          }
        })
        .catch(() => {});
    }

    const handleAccountsChanged = (accs: unknown) => {
      const list = accs as string[];
      if (!list || list.length === 0) {
        disconnect();
      } else {
        const next = list[0].toLowerCase();
        setAccount(next);
        setIsDemoMode(false);
        refreshBalance(next);
      }
    };

    const handleChainChanged = (c: unknown) => {
      setChainId(parseInt(c as string, 16));
    };

    eth.on?.("accountsChanged", handleAccountsChanged);
    eth.on?.("chainChanged", handleChainChanged);

    return () => {
      eth.removeListener?.("accountsChanged", handleAccountsChanged);
      eth.removeListener?.("chainChanged", handleChainChanged);
    };
  }, [hasInjected, disconnect, refreshBalance]);

  const isSepolia = chainId === SEPOLIA_CHAIN_ID || isDemoMode;

  return (
    <WalletContext.Provider
      value={{
        account,
        chainId,
        isSepolia,
        isConnecting,
        hasInjected,
        ethBalance,
        error,
        isDemoMode,
        connect,
        disconnect,
        switchNetwork,
        inspectAddress,
        clearError: () => setError(null),
      }}
    >
      {children}
    </WalletContext.Provider>
  );
}

export function useWallet() {
  const ctx = useContext(WalletContext);
  if (!ctx) throw new Error("useWallet must be used within a WalletProvider");
  return ctx;
}
