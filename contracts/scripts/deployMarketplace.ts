import { ethers } from "hardhat";
import * as fs from "fs";
import * as path from "path";

function writeAbi(contractName: string) {
  const artifactPath = path.join(
    __dirname,
    `../artifacts/contracts/${contractName}.sol/${contractName}.json`
  );
  const artifact = JSON.parse(fs.readFileSync(artifactPath, "utf-8"));
  const abiJson = JSON.stringify(artifact.abi, null, 2);

  for (const outDir of ["../../src-tauri/abi", "../../src/abi"]) {
    const outPath = path.join(__dirname, outDir, `${contractName}.abi.json`);
    fs.mkdirSync(path.dirname(outPath), { recursive: true });
    fs.writeFileSync(outPath, abiJson);
    console.log("Wrote ABI to", outPath);
  }
}

async function main() {
  const Voucher = await ethers.getContractFactory("CabalMeshVoucher");
  const voucher = await Voucher.deploy();
  await voucher.waitForDeployment();
  const voucherAddress = await voucher.getAddress();
  console.log("CabalMeshVoucher deployed to:", voucherAddress);

  const Marketplace = await ethers.getContractFactory("Marketplace");
  const marketplace = await Marketplace.deploy(voucherAddress);
  await marketplace.waitForDeployment();
  const marketplaceAddress = await marketplace.getAddress();
  console.log("Marketplace deployed to:", marketplaceAddress);

  writeAbi("CabalMeshVoucher");
  writeAbi("Marketplace");

  const network = await ethers.provider.getNetwork();
  const deploymentPath = path.join(__dirname, "../deployments/fuji.json");

  let existing: any = {};
  if (fs.existsSync(deploymentPath)) {
    existing = JSON.parse(fs.readFileSync(deploymentPath, "utf-8"));
  }

  const merged = {
    ...(existing.escrow ? { escrow: existing.escrow } : {}),
    voucher: { address: voucherAddress, chainId: Number(network.chainId), deployedAt: new Date().toISOString() },
    marketplace: { address: marketplaceAddress, chainId: Number(network.chainId), deployedAt: new Date().toISOString() },
  };

  fs.mkdirSync(path.dirname(deploymentPath), { recursive: true });
  fs.writeFileSync(deploymentPath, JSON.stringify(merged, null, 2));
  console.log("Wrote deployment info to", deploymentPath);
}

main().catch((error) => {
  console.error(error);
  process.exitCode = 1;
});
